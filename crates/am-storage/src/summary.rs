//! The overview: what the collection is worth, what it cost, and what needs
//! attention.
//!
//! Computed here in exact arithmetic rather than in the frontend, where the
//! only numeric type is a 53-bit float. Every figure keeps the same honesty
//! rules as the portfolio total:
//!
//! - an unpriced holding is **counted**, never treated as zero;
//! - gain is computed only over holdings with *both* a value and a known
//!   cost for the whole holding, and says how many that is — a gain over
//!   half the collection presented as the whole would be a confident lie;
//! - foreign-currency figures are excluded and named, not converted;
//! - a total too large to represent is an error, never a smaller number.

use am_core::{Currency, Money};
use serde::Serialize;

use crate::series::{format_date, parse_date};
use crate::vault::Vault;

#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    #[error(
        "the collection's total is too large to compute exactly — check for a \
         mistyped value"
    )]
    Overflow,
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

fn minor_as_string<S: serde::Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&v.to_string())
}

/// Money for the frontend: raw minor units as text, plus display form.
#[derive(Debug, Clone, Serialize)]
pub struct Amount {
    #[serde(serialize_with = "minor_as_string")]
    pub minor: i64,
    pub currency: String,
    pub display: String,
}

impl From<&Money> for Amount {
    fn from(m: &Money) -> Self {
        Amount {
            minor: m.amount_minor,
            currency: m.currency.code().to_string(),
            display: m.format(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryTotal {
    pub category: String,
    pub value: Amount,
    pub count: usize,
    /// Holdings in this category with no usable value.
    pub unvalued: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Brief {
    pub asset_id: String,
    pub name: String,
    pub type_id: String,
    pub category: String,
    pub type_label: String,
    pub value: Option<Amount>,
    /// Why this item is listed: "no value yet", "last valued 214 days ago".
    pub reason: String,
    pub primary_photo: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentEvent {
    pub asset_id: String,
    pub name: String,
    pub event_type: String,
    pub effective_date: String,
    pub quantity_delta: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Dashboard {
    pub currency: String,
    pub total: Amount,
    pub valued: usize,
    pub unvalued: usize,
    pub skipped_currencies: Vec<String>,
    /// Cost of the holdings that have both a value and a known cost.
    pub cost_basis: Amount,
    /// Value of those same holdings, so gain compares like with like.
    pub cost_covered_value: Amount,
    pub gain: Option<Amount>,
    /// How many holdings the gain is computed over.
    pub gain_coverage: usize,
    /// Valued holdings left out of the gain because part of the holding was
    /// added at an unknown cost.
    pub partial_cost: usize,
    pub active_count: usize,
    pub by_category: Vec<CategoryTotal>,
    pub top_holdings: Vec<Brief>,
    pub needs_value: Vec<Brief>,
    pub review_due: Vec<Brief>,
    pub recent: Vec<RecentEvent>,
}

struct Row {
    asset_id: String,
    name: String,
    type_id: String,
    type_label: String,
    category: String,
    value: Option<(i64, String)>,
    cost: Option<(i64, String)>,
    cost_complete: bool,
    value_asof: Option<String>,
    review_every_days: Option<i64>,
    primary_photo: Option<String>,
}

pub fn dashboard(
    vault: &Vault,
    currency: &Currency,
    today: &str,
) -> Result<Dashboard, SummaryError> {
    let rows: Vec<Row> = {
        let mut stmt = vault.conn().prepare(
            "SELECT a.asset_id, a.name, t.display_name, t.category,
                    a.current_amount_minor, a.current_currency,
                    a.acquired_amount_minor, a.acquired_currency,
                    a.value_asof, a.review_every_days, a.type_id, a.cost_complete,
                    (SELECT m.object_id FROM asset_media m
                       JOIN objects o ON o.object_id = m.object_id
                      WHERE m.asset_id = a.asset_id AND o.gc_state = 'live'
                      ORDER BY m.is_primary DESC, m.sort_order LIMIT 1)
             FROM assets a JOIN asset_types t ON t.type_id = a.type_id
             WHERE a.status = 'active' AND a.deleted_at IS NULL",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let pair = |a: Option<i64>, c: Option<String>| a.zip(c);
                Ok(Row {
                    asset_id: r.get(0)?,
                    name: r.get(1)?,
                    type_label: r.get(2)?,
                    category: r.get(3)?,
                    value: pair(r.get(4)?, r.get(5)?),
                    cost: pair(r.get(6)?, r.get(7)?),
                    value_asof: r.get(8)?,
                    review_every_days: r.get(9)?,
                    type_id: r.get(10)?,
                    cost_complete: r.get::<_, i64>(11)? != 0,
                    primary_photo: r.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    let zero = || Money::zero(currency.clone());
    let code = currency.code();
    let money = |minor: i64| Money::new(minor, currency.clone());

    let mut total = zero();
    let mut cost_basis = zero();
    let mut covered_value = zero();
    let (mut valued, mut unvalued, mut gain_coverage) = (0usize, 0usize, 0usize);
    let mut partial_cost = 0usize;
    let add = |sum: &Money, minor: i64| {
        sum.checked_add(&money(minor)).map_err(|_| SummaryError::Overflow)
    };
    let mut skipped: Vec<String> = Vec::new();
    let mut categories: Vec<(String, Money, usize, usize)> = Vec::new();
    let mut top: Vec<(i64, &Row)> = Vec::new();
    let mut needs_value = Vec::new();
    let mut review_due = Vec::new();
    let today_day = parse_date(today);

    let brief = |row: &Row, reason: String| Brief {
        asset_id: row.asset_id.clone(),
        name: row.name.clone(),
        type_id: row.type_id.clone(),
        category: row.category.clone(),
        type_label: row.type_label.clone(),
        value: row
            .value
            .as_ref()
            .and_then(|(m, c)| Currency::new(c).ok().map(|c| Amount::from(&Money::new(*m, c)))),
        reason,
        primary_photo: row.primary_photo.clone(),
    };

    for row in &rows {
        let slot = match categories.iter().position(|c| c.0 == row.category) {
            Some(i) => i,
            None => {
                categories.push((row.category.clone(), zero(), 0, 0));
                categories.len() - 1
            }
        };
        categories[slot].2 += 1;

        match &row.value {
            Some((minor, c)) if c == code => {
                valued += 1;
                total = add(&total, *minor)?;
                categories[slot].1 = add(&categories[slot].1, *minor)?;
                top.push((*minor, row));

                if let Some((cost, cost_code)) = &row.cost {
                    if cost_code == code && row.cost_complete {
                        gain_coverage += 1;
                        cost_basis = add(&cost_basis, *cost)?;
                        covered_value = add(&covered_value, *minor)?;
                    } else if cost_code == code {
                        partial_cost += 1;
                    }
                }
            }
            Some((_, c)) => {
                unvalued += 1;
                categories[slot].3 += 1;
                if !skipped.contains(c) {
                    skipped.push(c.clone());
                }
            }
            None => {
                unvalued += 1;
                categories[slot].3 += 1;
                needs_value.push(brief(row, "no value recorded yet".into()));
            }
        }

        // Review reminders: only for items the owner asked to be reminded
        // about, so the list is a to-do rather than a wall of nagging.
        if let Some(every) = row.review_every_days {
            let age = row
                .value_asof
                .as_deref()
                .and_then(parse_date)
                .zip(today_day)
                .map(|(asof, today)| today - asof);
            // An item with no value at all is already under "needs value".
            if let Some(days) = age.filter(|days| *days >= every) {
                review_due.push(brief(
                    row,
                    format!("last valued {days} days ago — review every {every}"),
                ));
            }
        }
    }

    top.sort_by(|a, b| b.0.cmp(&a.0));
    let top_holdings = top.iter().take(6).map(|(_, row)| brief(row, String::new())).collect();

    categories.sort_by(|a, b| b.1.amount_minor.cmp(&a.1.amount_minor));
    let by_category = categories
        .into_iter()
        .map(|(category, value, count, unvalued)| CategoryTotal {
            category,
            value: Amount::from(&value),
            count,
            unvalued,
        })
        .collect();

    let gain = if gain_coverage > 0 {
        let gain =
            covered_value.checked_sub(&cost_basis).map_err(|_| SummaryError::Overflow)?;
        Some(Amount::from(&gain))
    } else {
        None
    };

    let recent = {
        let mut stmt = vault.conn().prepare(
            "SELECT e.asset_id, a.name, e.event_type, e.effective_date, e.quantity_delta
             FROM asset_events e JOIN assets a ON a.asset_id = e.asset_id
             WHERE a.deleted_at IS NULL
             ORDER BY e.recorded_at DESC, e.rowid DESC LIMIT 8",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(RecentEvent {
                    asset_id: r.get(0)?,
                    name: r.get(1)?,
                    event_type: r.get(2)?,
                    effective_date: r.get(3)?,
                    quantity_delta: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    skipped.sort();
    needs_value.truncate(20);
    Ok(Dashboard {
        currency: code.to_string(),
        total: Amount::from(&total),
        valued,
        unvalued,
        skipped_currencies: skipped,
        cost_basis: Amount::from(&cost_basis),
        cost_covered_value: Amount::from(&covered_value),
        gain,
        gain_coverage,
        partial_cost,
        active_count: rows.len(),
        by_category,
        top_holdings,
        needs_value,
        review_due,
        recent,
    })
}

/// A date `days` after `date`, for "next review on".
pub fn add_days(date: &str, days: i64) -> Option<String> {
    parse_date(date).map(|d| format_date(d + days))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{self, NewAsset, Pricing};
    use crate::valuations::{record_valuation, Basis, NewValuation, Provenance};
    use am_core::Decimal;
    use am_crypto::KdfParams;
    use std::collections::BTreeMap;

    const NOW: &str = "2026-09-19T10:00:00Z";

    fn usd() -> Currency {
        Currency::new("USD").unwrap()
    }

    fn setup() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let fast = KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 };
        let (v, _r) =
            Vault::create(&dir.path().join("v"), "correct horse battery staple", &fast, NOW)
                .unwrap();
        (dir, v)
    }

    fn asset(v: &Vault, name: &str, type_id: &str, cost: Option<i64>) -> String {
        assets::create(
            v,
            &NewAsset {
                type_id: type_id.into(),
                name: name.into(),
                quantity: Decimal::ONE,
                quantity_unit: "item".into(),
                acquired_date: Some("2026-01-01".into()),
                effective_date: None,
                acquired_cost: cost.map(|c| Money::new(c, usd())),
                acquired_from: None,
                storage_location: None,
                notes: String::new(),
                insured: None,
                attrs: BTreeMap::new(),
                pricing: Pricing::Manual,
                review_every_days: None,
            },
            NOW,
        )
        .unwrap()
    }

    fn value(v: &Vault, id: &str, minor: i64, code: &str, asof: &str) {
        record_valuation(
            v,
            &NewValuation {
                asset_id: id.into(),
                quote_id: None,
                value: Money::new(minor, Currency::new(code).unwrap()),
                quantity_at_time: Decimal::ONE,
                basis: Basis::EstimatedResale,
                provenance: Provenance::Manual,
                inputs: serde_json::json!({}),
                asof: asof.into(),
            },
            NOW,
        )
        .unwrap();
    }

    #[test]
    fn gain_is_computed_only_where_cost_is_known() {
        let (_d, v) = setup();
        let a = asset(&v, "Watch", "watch", Some(500_000));
        let b = asset(&v, "Comic", "comic", None); // cost unknown
        let _c = asset(&v, "Unpriced card", "trading_card", Some(10_000));
        value(&v, &a, 650_000, "USD", "2026-09-01");
        value(&v, &b, 90_000, "USD", "2026-09-01");

        let d = dashboard(&v, &usd(), "2026-09-19").unwrap();
        assert_eq!(d.total.minor, 740_000);
        assert_eq!((d.valued, d.unvalued), (2, 1), "the unpriced card is counted");
        assert_eq!(d.gain_coverage, 1, "only the watch has both a cost and a value");
        assert_eq!(d.gain.unwrap().minor, 150_000, "not 740k − 510k");
        assert_eq!(d.needs_value.len(), 1);
        assert_eq!(d.needs_value[0].name, "Unpriced card");
    }

    #[test]
    fn categories_sum_to_the_total_and_foreign_currency_is_named() {
        let (_d, v) = setup();
        let a = asset(&v, "Watch", "watch", None);
        let b = asset(&v, "Painting", "art", None);
        let c = asset(&v, "Card", "trading_card", None);
        value(&v, &a, 100_000, "USD", "2026-09-01");
        value(&v, &b, 300_000, "USD", "2026-09-01");
        value(&v, &c, 5_000, "EUR", "2026-09-01");

        let d = dashboard(&v, &usd(), "2026-09-19").unwrap();
        assert_eq!(d.skipped_currencies, vec!["EUR".to_string()]);
        let sum: i64 = d.by_category.iter().map(|c| c.value.minor).sum();
        assert_eq!(sum, d.total.minor);
        assert_eq!(d.by_category[0].category, "valuables", "largest first");
        assert_eq!(d.top_holdings[0].name, "Painting");
    }

    #[test]
    fn review_reminders_fire_only_when_due() {
        let (_d, v) = setup();
        let due = asset(&v, "Due", "art", None);
        let fresh = asset(&v, "Fresh", "art", None);
        for id in [&due, &fresh] {
            v.conn()
                .execute("UPDATE assets SET review_every_days = 90 WHERE asset_id = ?1", [id])
                .unwrap();
        }
        value(&v, &due, 1_000, "USD", "2026-05-01");
        value(&v, &fresh, 1_000, "USD", "2026-09-01");

        let d = dashboard(&v, &usd(), "2026-09-19").unwrap();
        assert_eq!(d.review_due.len(), 1);
        assert_eq!(d.review_due[0].name, "Due");
        assert!(d.review_due[0].reason.contains("141 days"), "{}", d.review_due[0].reason);
        assert_eq!(add_days("2026-05-01", 90).as_deref(), Some("2026-07-30"));
    }

    #[test]
    fn a_cost_covering_part_of_a_holding_gives_no_gain() {
        // Buy one for $100, then another at an unknown price: the $100 is
        // the cost of half the holding, so there is no honest gain figure.
        let (_d, v) = setup();
        let a = asset(&v, "Coin", "generic", Some(10_000));
        value(&v, &a, 10_000, "USD", "2026-02-01");
        crate::events::record(
            &v,
            &crate::events::NewEvent {
                asset_id: a.clone(),
                event_type: crate::events::EventType::Add,
                effective_date: "2026-03-01".into(),
                quantity_delta: Decimal::ONE,
                amount_minor: None,
                currency: None,
                note: String::new(),
            },
            NOW,
        )
        .unwrap();

        let d = dashboard(&v, &usd(), "2026-09-19").unwrap();
        assert!(d.gain.is_none(), "no gain against a partial cost");
        assert_eq!(d.gain_coverage, 0);
        assert_eq!(d.partial_cost, 1, "and the reason is counted");
    }

    #[test]
    fn a_total_too_large_to_represent_is_an_error_not_a_smaller_number() {
        let (_d, v) = setup();
        let a = asset(&v, "A", "art", None);
        let b = asset(&v, "B", "art", None);
        value(&v, &a, i64::MAX / 2 + 1, "USD", "2026-09-01");
        value(&v, &b, i64::MAX / 2 + 1, "USD", "2026-09-01");
        assert!(matches!(dashboard(&v, &usd(), "2026-09-19"), Err(SummaryError::Overflow)));
    }
}
