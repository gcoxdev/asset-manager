//! Exchange rates and conversion.
//!
//! Policy, in one place: an amount is converted at the latest rate on record
//! on or before the date of the figure it is part of. A rate entered as
//! EUR→USD also serves USD→EUR (its inverse). With no rate, the amount is
//! not converted, and the figure says so rather than treating it as zero.

use am_core::{Currency, Decimal, Money};
use serde::Serialize;

use crate::vault::Vault;

#[derive(Debug, thiserror::Error)]
pub enum FxError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Rate {
    pub rate_id: String,
    pub from_currency: String,
    pub to_currency: String,
    /// 1 `from_currency` = `rate` `to_currency`, as a decimal string.
    pub rate: String,
    pub asof: String,
    pub source: String,
}

/// The rate used for one conversion, for saying how a total was reached.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Applied {
    pub from_currency: String,
    pub to_currency: String,
    pub rate: String,
    pub asof: String,
    /// True when the stored rate was the other way round and inverted.
    pub inverted: bool,
}

pub fn record(
    vault: &Vault,
    from: &str,
    to: &str,
    rate: &str,
    asof: &str,
    today: &str,
    now: &str,
) -> Result<String, FxError> {
    let from = Currency::new(from.trim()).map_err(|e| FxError::Invalid(e.to_string()))?;
    let to = Currency::new(to.trim()).map_err(|e| FxError::Invalid(e.to_string()))?;
    if from == to {
        return Err(FxError::Invalid("choose two different currencies".into()));
    }
    let value = am_core::parse_decimal(&rate.trim().replace(',', ""))
        .map_err(|_| FxError::Invalid(format!("{rate:?} is not a rate")))?;
    if value <= Decimal::ZERO {
        return Err(FxError::Invalid("a rate must be more than zero".into()));
    }
    let asof =
        crate::events::normalize_date(asof).map_err(|e| FxError::Invalid(e.to_string()))?;
    if asof.as_str() > today {
        return Err(FxError::Invalid("a rate cannot be dated in the future".into()));
    }
    let id = new_id();
    vault.conn().execute(
        "INSERT INTO fx_rates (rate_id, from_currency, to_currency, rate, asof, source, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'manual', ?6)",
        rusqlite::params![&id, from.code(), to.code(), value.normalize().to_string(), &asof, now],
    )?;
    Ok(id)
}

pub fn delete(vault: &Vault, rate_id: &str) -> Result<(), FxError> {
    let n = vault.conn().execute("DELETE FROM fx_rates WHERE rate_id = ?1", [rate_id])?;
    if n == 0 {
        return Err(FxError::Invalid("that rate no longer exists".into()));
    }
    Ok(())
}

/// Every rate on record, newest first.
pub fn list(vault: &Vault) -> Result<Vec<Rate>, FxError> {
    let mut stmt = vault.conn().prepare(
        "SELECT rate_id, from_currency, to_currency, rate, asof, source FROM fx_rates
         ORDER BY asof DESC, recorded_at DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Rate {
                rate_id: r.get(0)?,
                from_currency: r.get(1)?,
                to_currency: r.get(2)?,
                rate: r.get(3)?,
                asof: r.get(4)?,
                source: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn latest(
    conn: &rusqlite::Connection,
    from: &str,
    to: &str,
    date: &str,
) -> Result<Option<(Decimal, String, String)>, FxError> {
    let row: Option<(String, String, String)> = conn
        .query_row(
            "SELECT rate, asof, recorded_at FROM fx_rates
             WHERE from_currency = ?1 AND to_currency = ?2 AND asof <= ?3
             ORDER BY asof DESC, recorded_at DESC LIMIT 1",
            [from, to, date],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    row.map(|(rate, asof, recorded)| {
        am_core::parse_decimal(&rate)
            .map(|r| (r, asof, recorded))
            .map_err(|_| FxError::Invalid(format!("stored rate {rate:?} is malformed")))
    })
    .transpose()
}

/// The rate from one currency to another in effect on a date: the most
/// recent of the direct rate and the inverse of the reverse one.
pub fn rate_on(
    conn: &rusqlite::Connection,
    from: &str,
    to: &str,
    date: &str,
) -> Result<Option<(Decimal, Applied)>, FxError> {
    if from == to {
        return Ok(Some((
            Decimal::ONE,
            Applied {
                from_currency: from.into(),
                to_currency: to.into(),
                rate: "1".into(),
                asof: date.into(),
                inverted: false,
            },
        )));
    }
    let direct = latest(conn, from, to, date)?;
    let reverse = latest(conn, to, from, date)?;
    let pick = match (direct, reverse) {
        (Some(d), Some(r)) => {
            if (r.1.as_str(), r.2.as_str()) > (d.1.as_str(), d.2.as_str()) {
                Some((Decimal::ONE / r.0, r.1, true))
            } else {
                Some((d.0, d.1, false))
            }
        }
        (Some(d), None) => Some((d.0, d.1, false)),
        (None, Some(r)) => Some((Decimal::ONE / r.0, r.1, true)),
        (None, None) => None,
    };
    Ok(pick.map(|(rate, asof, inverted)| {
        let shown = rate.round_dp(8).normalize().to_string();
        (
            rate,
            Applied {
                from_currency: from.into(),
                to_currency: to.into(),
                rate: shown,
                asof,
                inverted,
            },
        )
    }))
}

/// Convert an amount into `to` at the rate in effect on `date`, rounded to
/// `to`'s minor units. `None` when no rate is on record.
pub fn convert(
    conn: &rusqlite::Connection,
    money: &Money,
    to: &Currency,
    date: &str,
) -> Result<Option<(Money, Applied)>, FxError> {
    let Some((rate, applied)) = rate_on(conn, money.currency.code(), to.code(), date)? else {
        return Ok(None);
    };
    let converted = Money::from_total_decimal(money.to_decimal() * rate, to.clone())
        .map_err(|e| FxError::Invalid(e.to_string()))?;
    Ok(Some((converted, applied)))
}

fn new_id() -> String {
    let mut raw = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
    raw.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_crypto::KdfParams;

    const NOW: &str = "2026-09-19T00:00:00Z";
    const TODAY: &str = "2026-09-19";

    fn setup() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let fast = KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 };
        let (v, _r) =
            Vault::create(&dir.path().join("v"), "correct horse battery staple", &fast, NOW)
                .unwrap();
        (dir, v)
    }

    fn c(code: &str) -> Currency {
        Currency::new(code).unwrap()
    }

    #[test]
    fn conversion_uses_the_rate_in_effect_on_the_date() {
        let (_d, v) = setup();
        record(&v, "EUR", "USD", "1.10", "2026-01-01", TODAY, NOW).unwrap();
        record(&v, "EUR", "USD", "1.20", "2026-06-01", TODAY, NOW).unwrap();
        let eur = Money::new(10_000, c("EUR")); // €100.00

        let (march, applied) =
            convert(v.conn(), &eur, &c("USD"), "2026-03-01").unwrap().unwrap();
        assert_eq!(march.amount_minor, 11_000);
        assert_eq!(applied.asof, "2026-01-01");
        let (today, _) = convert(v.conn(), &eur, &c("USD"), TODAY).unwrap().unwrap();
        assert_eq!(today.amount_minor, 12_000);
        assert!(
            convert(v.conn(), &eur, &c("USD"), "2025-12-31").unwrap().is_none(),
            "no rate yet: not converted"
        );
    }

    #[test]
    fn a_rate_serves_both_directions_and_minor_units_differ() {
        let (_d, v) = setup();
        record(&v, "USD", "JPY", "150", "2026-09-01", TODAY, NOW).unwrap();
        let yen = Money::new(15_000, c("JPY")); // ¥15,000
        let (usd, applied) = convert(v.conn(), &yen, &c("USD"), TODAY).unwrap().unwrap();
        assert_eq!(usd.amount_minor, 10_000, "$100.00");
        assert!(applied.inverted);
        let (back, _) = convert(v.conn(), &usd, &c("JPY"), TODAY).unwrap().unwrap();
        assert_eq!(back.amount_minor, 15_000);
    }

    #[test]
    fn rates_are_validated() {
        let (_d, v) = setup();
        assert!(record(&v, "EUR", "EUR", "1", TODAY, TODAY, NOW).is_err());
        assert!(record(&v, "EUR", "USD", "0", TODAY, TODAY, NOW).is_err());
        assert!(record(&v, "EUR", "USD", "lots", TODAY, TODAY, NOW).is_err());
        assert!(record(&v, "EUR", "USD", "1.1", "2026-12-01", TODAY, NOW).is_err());
        assert!(record(&v, "EURO", "USD", "1.1", TODAY, TODAY, NOW).is_err());
    }
}
