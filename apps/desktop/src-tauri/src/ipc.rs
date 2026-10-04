//! Helpers shared by every command module.
//!
//! Money and decimals cross IPC as **strings** in both directions. JavaScript
//! numbers are 53-bit floats, so a large minor-unit amount or an 18-decimal
//! crypto quantity would corrupt silently if either side parsed it as one.

use am_core::{parse_decimal, Currency, Money};
use am_storage::vault::{Vault, VaultError};

use crate::session::{IpcError, SessionError};

pub type IpcResult<T> = Result<T, IpcError>;

/// ISO-8601 UTC, whole seconds.
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Today as `YYYY-MM-DD`, in UTC to match every stored timestamp.
pub fn today() -> String {
    now()[..10].to_string()
}

pub fn bad_input(message: impl Into<String>) -> IpcError {
    IpcError { kind: "invalid_input".into(), message: message.into() }
}

pub fn storage(e: impl std::fmt::Display) -> SessionError {
    SessionError::Vault(VaultError::Other(e.to_string()))
}

/// Setting key for the currency new values default to and totals are shown in.
pub const CURRENCY_SETTING: &str = "currency";

/// The vault's base currency. USD unless the owner has chosen otherwise.
pub fn base_currency(vault: &Vault) -> Currency {
    am_storage::settings::get(vault, CURRENCY_SETTING)
        .ok()
        .flatten()
        .and_then(|code| Currency::new(&code).ok())
        .unwrap_or_else(|| Currency::new("USD").expect("USD is a valid code"))
}

/// A price as typed by a person, in major units: "1299.50", "1,299.50", "$1,299.50".
///
/// Parsed as exact decimal and converted to minor units once, at the end.
/// Grouping commas and a leading currency symbol are tolerated because that
/// is how people copy prices from a listing; anything else is refused.
pub fn parse_money(amount: &str, currency: &Currency) -> Result<Money, IpcError> {
    let cleaned: String = amount
        .trim()
        .trim_start_matches(|c: char| !c.is_ascii_digit() && c != '-' && c != '.')
        .chars()
        .filter(|c| *c != ',' && *c != '_' && !c.is_whitespace())
        .collect();
    let decimal = parse_decimal(&cleaned)
        .map_err(|_| bad_input(format!("{amount:?} is not a valid amount")))?;
    if decimal.is_sign_negative() {
        return Err(bad_input("an amount cannot be negative"));
    }
    Money::from_total_decimal(decimal, currency.clone())
        .map_err(|e| bad_input(format!("amount: {e}")))
}

/// Optional money: blank means "not recorded", which is not zero.
pub fn parse_money_opt(
    amount: &Option<String>,
    currency: &Currency,
) -> Result<Option<Money>, IpcError> {
    match amount.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(text) => parse_money(text, currency).map(Some),
    }
}

/// A currency code from the frontend, or the vault's base currency.
pub fn currency_or(code: &Option<String>, fallback: &Currency) -> Result<Currency, IpcError> {
    match code.as_deref().map(str::trim) {
        None | Some("") => Ok(fallback.clone()),
        Some(c) => Currency::new(c).map_err(|e| bad_input(e.to_string())),
    }
}

/// Format stored minor units for display, respecting the currency's digits.
pub fn format_money(amount_minor: Option<i64>, currency: Option<&str>) -> Option<String> {
    let (amount, code) = (amount_minor?, currency?);
    Currency::new(code).ok().map(|c| Money::new(amount, c).format())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usd() -> Currency {
        Currency::new("USD").unwrap()
    }

    #[test]
    fn money_is_parsed_exactly_not_through_a_float() {
        assert_eq!(parse_money("1299.50", &usd()).unwrap().amount_minor, 129_950);
        // A value f64 cannot represent exactly.
        assert_eq!(parse_money("0.07", &usd()).unwrap().amount_minor, 7);
    }

    #[test]
    fn pasted_prices_are_accepted() {
        assert_eq!(parse_money("$1,299.50", &usd()).unwrap().amount_minor, 129_950);
        assert_eq!(parse_money(" 12 500 ", &usd()).unwrap().amount_minor, 1_250_000);
        assert_eq!(
            parse_money("€45", &Currency::new("EUR").unwrap()).unwrap().amount_minor,
            4500
        );
    }

    #[test]
    fn nonsense_and_negatives_are_refused() {
        for bad in ["", "abc", "12.3.4", "-5", "1e5x"] {
            assert!(parse_money(bad, &usd()).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn zero_decimal_currencies_are_handled() {
        let yen = parse_money("1000", &Currency::new("JPY").unwrap()).unwrap();
        assert_eq!(yen.amount_minor, 1000, "JPY has no minor units");
    }

    #[test]
    fn blank_is_unknown_not_zero() {
        assert_eq!(parse_money_opt(&None, &usd()).unwrap(), None);
        assert_eq!(parse_money_opt(&Some("  ".into()), &usd()).unwrap(), None);
        assert!(parse_money_opt(&Some("0".into()), &usd()).unwrap().is_some());
    }
}
