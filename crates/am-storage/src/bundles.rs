//! Splitting a holding, sets of items, and dividing one purchase among
//! several items.

use am_core::{Currency, Decimal, Money};
use serde::Serialize;

use crate::assets::{self, AssetError, NewAsset, Pricing};
use crate::events::{self, EventType, NewEvent};
use crate::vault::Vault;

fn invalid(m: impl Into<String>) -> AssetError {
    AssetError::Other(m.into())
}

fn share(minor: i64, part: Decimal, whole: Decimal) -> Result<i64, AssetError> {
    crate::valuations::scale_to_quantity(
        &Money::new(minor, Currency::new("USD").unwrap()),
        whole,
        part,
    )
    .map(|m| m.amount_minor)
    .map_err(|e| invalid(e.to_string()))
}

/// Make `quantity` of a holding into an item of its own, named `new_name`.
///
/// The original records a split (not a sale) on `today`; the new item is
/// acquired the same day with the original's acquisition date shown, its
/// share of the cost and — for a hand-valued holding — its share of the
/// value. Totals on every date are unchanged by the split. Identifiers
/// (serials, certificates) stay with the original. Returns the new ID.
pub fn split(
    vault: &Vault,
    asset_id: &str,
    quantity: Decimal,
    new_name: &str,
    today: &str,
    now: &str,
) -> Result<String, AssetError> {
    let original = assets::get(vault, asset_id)?;
    if original.status != "active" || original.deleted_at.is_some() {
        return Err(invalid("only a held item can be split"));
    }
    let held = events::quantity_as_of(vault, asset_id, Some(today))?;
    if quantity <= Decimal::ZERO || quantity >= held {
        return Err(invalid(format!(
            "split off more than none and less than all {} held",
            held.normalize()
        )));
    }
    let name = new_name.trim();
    if name.is_empty() {
        return Err(invalid("give the new item a name"));
    }
    let future: i64 = vault.conn().query_row(
        "SELECT count(*) FROM asset_events WHERE asset_id = ?1 AND effective_date > ?2",
        [asset_id, today],
        |r| r.get(0),
    )?;
    if future != 0 {
        return Err(invalid(
            "resolve future-dated quantity changes before splitting this holding",
        ));
    }
    let cost = events::derive_cost(vault.conn(), asset_id)?;
    let value = original.current_amount_minor.zip(original.current_currency.clone());

    let unit = crate::atomic::begin(vault.conn())?;
    events::record(
        vault,
        &NewEvent {
            asset_id: asset_id.to_string(),
            event_type: EventType::Split,
            effective_date: today.to_string(),
            quantity_delta: -quantity,
            amount_minor: None,
            currency: None,
            note: format!("Split off as “{name}”"),
        },
        now,
    )?;
    let allocated = match &cost.amount {
        Some((minor, code)) => Some(Money::new(
            share(*minor, quantity, held)?,
            Currency::new(code).map_err(|e| invalid(e.to_string()))?,
        )),
        _ => None,
    };
    // Allocate one rounded share; keep its exact complement in dated history.
    if let Some((minor, code)) = &cost.amount {
        let remainder = minor
            .checked_sub(share(*minor, quantity, held)?)
            .ok_or_else(|| invalid("split cost overflows"))?;
        unit.execute(
            "INSERT INTO cost_statements (statement_id, asset_id, effective_date, amount_minor,
                currency, covers_holding, note, recorded_at) VALUES (?1, ?2, ?3, ?4, ?5, ?7, 'Remainder after split', ?6)",
            rusqlite::params![random_id(), asset_id, today, remainder, code, now, cost.complete],
        )?;
        events::rebuild_cost_in(&unit, asset_id)?;
    }
    let attrs = original
        .attrs
        .iter()
        .filter(|(k, _)| !crate::organize::IDENTIFYING_KEYS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let market = original.pricing == "market";
    let new_id = assets::create(
        vault,
        &NewAsset {
            type_id: original.type_id.clone(),
            name: name.to_string(),
            quantity,
            quantity_unit: original.quantity_unit.clone(),
            // Held from the split, not from the original purchase: before
            // today these units are counted in the original.
            acquired_date: None,
            effective_date: Some(today.to_string()),
            acquired_cost: allocated,
            acquired_from: original.acquired_from.clone(),
            storage_location: original.storage_location.clone(),
            notes: format!("Split from “{}” on {today}.", original.name),
            insured: None,
            attrs,
            pricing: if market { Pricing::Market } else { Pricing::Manual },
            review_every_days: original.review_every_days,
        },
        now,
    )?;
    if let (Some((minor, code)), false) = (&cost.amount, cost.complete) {
        unit.execute("INSERT INTO cost_statements (statement_id, asset_id, effective_date, amount_minor,
            currency, covers_holding, note, recorded_at) VALUES (?1, ?2, ?3, ?4, ?5, 0, 'Partial known cost allocated by split', ?6)",
            rusqlite::params![random_id(), &new_id, today, share(*minor, quantity, held)?, code, now])?;
        events::rebuild_cost_in(&unit, &new_id)?;
    }
    // Show when the units were first acquired, without moving the acquire
    // event (which would count them twice before the split).
    unit.execute(
        "UPDATE assets SET acquired_date = ?1 WHERE asset_id = ?2",
        rusqlite::params![&original.acquired_date, &new_id],
    )?;
    unit.execute(
        "INSERT INTO asset_tags (asset_id, tag_id) SELECT ?1, tag_id FROM asset_tags WHERE asset_id = ?2",
        [&new_id, asset_id],
    )?;
    if let (Some((minor, code)), false) = (&value, market) {
        crate::valuations::record_valuation(
            vault,
            &crate::valuations::NewValuation {
                asset_id: asset_id.to_string(),
                quote_id: None,
                value: Money::new(
                    minor
                        .checked_sub(share(*minor, quantity, held)?)
                        .ok_or_else(|| invalid("split value overflows"))?,
                    Currency::new(code).map_err(|e| invalid(e.to_string()))?,
                ),
                quantity_at_time: held - quantity,
                basis: crate::valuations::Basis::EstimatedResale,
                provenance: crate::valuations::Provenance::Manual,
                inputs: serde_json::json!({"note":"Remainder after split"}),
                asof: today.to_string(),
            },
            now,
        )
        .map_err(|e| invalid(e.to_string()))?;

        crate::valuations::record_valuation(
            vault,
            &crate::valuations::NewValuation {
                asset_id: new_id.clone(),
                quote_id: None,
                value: Money::new(
                    share(*minor, quantity, held)?,
                    Currency::new(code).map_err(|e| invalid(e.to_string()))?,
                ),
                quantity_at_time: quantity,
                basis: crate::valuations::Basis::EstimatedResale,
                provenance: crate::valuations::Provenance::Manual,
                inputs: serde_json::json!({ "note": format!("Share of “{}” when split", original.name) }),
                asof: today.to_string(),
            },
            now,
        )
        .map_err(|e| invalid(e.to_string()))?;
    }
    unit.commit()?;
    Ok(new_id)
}

// ---------------------------------------------------------------- sets

#[derive(Debug, Clone, Serialize)]
pub struct SetSummary {
    pub set_id: String,
    pub name: String,
    pub target_count: Option<i64>,
    pub notes: String,
    pub members: i64,
    /// Members' current values in the base currency (converted where a rate
    /// is on record), and how many had none.
    pub value_minor: i64,
    pub value_display: String,
    pub unvalued: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetMember {
    pub asset_id: String,
    pub name: String,
    pub type_label: String,
    pub status: String,
    pub current_minor: Option<i64>,
    pub current_currency: Option<String>,
    /// The current value formatted for display, e.g. "12.50 USD".
    pub current_display: Option<String>,
}

pub fn create_set(
    vault: &Vault,
    name: &str,
    target_count: Option<i64>,
    notes: &str,
    now: &str,
) -> Result<String, AssetError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(invalid("a set needs a name"));
    }
    if target_count.is_some_and(|t| t <= 0 || t > 100_000) {
        return Err(invalid("the target is a positive number of items"));
    }
    let id = random_id();
    vault.conn().execute(
        "INSERT INTO sets (set_id, name, target_count, notes, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![&id, name, target_count, notes.trim(), now],
    )?;
    Ok(id)
}

pub fn update_set(
    vault: &Vault,
    set_id: &str,
    name: &str,
    target_count: Option<i64>,
    notes: &str,
) -> Result<(), AssetError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(invalid("a set needs a name"));
    }
    if target_count.is_some_and(|t| t <= 0 || t > 100_000) {
        return Err(invalid("the target is a positive number of items"));
    }
    let n = vault.conn().execute(
        "UPDATE sets SET name = ?1, target_count = ?2, notes = ?3 WHERE set_id = ?4",
        rusqlite::params![name, target_count, notes.trim(), set_id],
    )?;
    if n == 0 {
        return Err(invalid("that set no longer exists"));
    }
    Ok(())
}

/// Delete a set. Its members are untouched.
pub fn delete_set(vault: &Vault, set_id: &str) -> Result<(), AssetError> {
    vault.conn().execute("DELETE FROM sets WHERE set_id = ?1", [set_id])?;
    Ok(())
}

pub fn add_members(
    vault: &Vault,
    set_id: &str,
    asset_ids: &[String],
    now: &str,
) -> Result<usize, AssetError> {
    let exists: i64 = vault.conn().query_row(
        "SELECT count(*) FROM sets WHERE set_id = ?1",
        [set_id],
        |r| r.get(0),
    )?;
    if exists == 0 {
        return Err(invalid("that set no longer exists"));
    }
    let unit = crate::atomic::begin(vault.conn())?;
    let mut added = 0;
    for id in asset_ids {
        assets::get(vault, id)?;
        added += unit.execute(
            "INSERT OR IGNORE INTO set_members (set_id, asset_id, added_at) VALUES (?1, ?2, ?3)",
            [set_id, id, now],
        )?;
    }
    unit.commit()?;
    Ok(added)
}

pub fn remove_member(vault: &Vault, set_id: &str, asset_id: &str) -> Result<(), AssetError> {
    vault.conn().execute(
        "DELETE FROM set_members WHERE set_id = ?1 AND asset_id = ?2",
        [set_id, asset_id],
    )?;
    Ok(())
}

/// Members of a set, as they stand: trashed items are left out.
pub fn members(vault: &Vault, set_id: &str) -> Result<Vec<SetMember>, AssetError> {
    let mut stmt = vault.conn().prepare(
        "SELECT a.asset_id, a.name, t.display_name, a.status, a.current_amount_minor, a.current_currency
         FROM set_members s JOIN assets a ON a.asset_id = s.asset_id JOIN asset_types t ON t.type_id = a.type_id
         WHERE s.set_id = ?1 AND a.deleted_at IS NULL ORDER BY a.name COLLATE NOCASE",
    )?;
    let rows = stmt
        .query_map([set_id], |r| {
            let current_minor: Option<i64> = r.get(4)?;
            let current_currency: Option<String> = r.get(5)?;
            let current_display =
                current_minor.zip(current_currency.as_deref()).and_then(|(minor, code)| {
                    Some(Money::new(minor, Currency::new(code).ok()?).format())
                });
            Ok(SetMember {
                asset_id: r.get(0)?,
                name: r.get(1)?,
                type_label: r.get(2)?,
                status: r.get(3)?,
                current_minor,
                current_currency,
                current_display,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Every set with its completion and combined value in `currency`.
pub fn sets(
    vault: &Vault,
    currency: &Currency,
    today: &str,
) -> Result<Vec<SetSummary>, AssetError> {
    let rows: Vec<(String, String, Option<i64>, String)> = {
        let mut stmt = vault.conn().prepare(
            "SELECT set_id, name, target_count, notes FROM sets ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<Result<_, _>>()?;
        rows
    };
    let mut out = Vec::new();
    for (set_id, name, target_count, notes) in rows {
        let list = members(vault, &set_id)?;
        let mut total = Money::zero(currency.clone());
        let mut unvalued = 0;
        for m in list.iter().filter(|m| m.status == "active") {
            let converted = match (m.current_minor, m.current_currency.as_deref()) {
                (Some(minor), Some(code)) => {
                    let money = Money::new(
                        minor,
                        Currency::new(code).map_err(|e| invalid(e.to_string()))?,
                    );
                    crate::fx::convert(vault.conn(), &money, currency, today)
                        .map_err(|e| invalid(e.to_string()))?
                        .map(|(m, _)| m)
                }
                _ => None,
            };
            match converted {
                Some(v) => total = total.checked_add(&v).map_err(|e| invalid(e.to_string()))?,
                None => unvalued += 1,
            }
        }
        out.push(SetSummary {
            members: list.len() as i64,
            value_minor: total.amount_minor,
            value_display: total.format(),
            unvalued,
            set_id,
            name,
            target_count,
            notes,
        });
    }
    Ok(out)
}

/// Divide one purchase's price among the items it bought — equally, or in
/// proportion to their current values — so the shares add up to the cent.
/// Each item's cost is restated (and so marked complete).
pub fn allocate_cost(
    vault: &Vault,
    asset_ids: &[String],
    total: &Money,
    by_value: bool,
    now: &str,
) -> Result<Vec<(String, i64)>, AssetError> {
    if asset_ids.len() < 2 || asset_ids.len() > 5_000 {
        return Err(invalid("choose at least two items"));
    }
    if total.amount_minor < 0 {
        return Err(invalid("the price cannot be negative"));
    }
    let weights: Vec<Decimal> = if by_value {
        asset_ids
            .iter()
            .map(|id| {
                let r = assets::get(vault, id)?;
                match (r.current_amount_minor, r.current_currency.as_deref()) {
                    (Some(m), Some(c)) if c == total.currency.code() && m > 0 => {
                        Ok(Decimal::from(m))
                    }
                    _ => Err(invalid(format!(
                        "{} has no value in {} to divide by — divide equally instead",
                        r.name,
                        total.currency.code()
                    ))),
                }
            })
            .collect::<Result<_, _>>()?
    } else {
        vec![Decimal::ONE; asset_ids.len()]
    };
    let sum: Decimal = weights.iter().copied().sum();
    // Round each share down, then hand out the leftover cents to the largest
    // remainders, so the shares always add up to the price exactly.
    let exact: Vec<Decimal> =
        weights.iter().map(|w| Decimal::from(total.amount_minor) * *w / sum).collect();
    let mut shares: Vec<i64> =
        exact.iter().map(|e| e.floor().try_into().unwrap_or(0)).collect();
    let mut leftover = total.amount_minor - shares.iter().sum::<i64>();
    let mut order: Vec<usize> = (0..exact.len()).collect();
    order.sort_by(|a, b| (exact[*b] - exact[*b].floor()).cmp(&(exact[*a] - exact[*a].floor())));
    for i in order {
        if leftover == 0 {
            break;
        }
        shares[i] += 1;
        leftover -= 1;
    }
    let unit = crate::atomic::begin(vault.conn())?;
    for (id, share) in asset_ids.iter().zip(&shares) {
        events::restate_cost_in(
            &unit,
            id,
            Some((*share, total.currency.code().to_string())),
            "Share of a purchase",
            now,
        )?;
    }
    unit.commit()?;
    Ok(asset_ids.iter().cloned().zip(shares).collect())
}

fn random_id() -> String {
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

    fn usd(minor: i64) -> Money {
        Money::new(minor, Currency::new("USD").unwrap())
    }

    fn setup() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let fast = KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 };
        let (v, _r) =
            Vault::create(&dir.path().join("v"), "correct horse battery staple", &fast, NOW)
                .unwrap();
        (dir, v)
    }

    fn holding(v: &Vault, name: &str, quantity: i64, cost: Option<i64>) -> String {
        assets::create(
            v,
            &NewAsset {
                type_id: "numismatic_coin".into(),
                name: name.into(),
                quantity: Decimal::from(quantity),
                quantity_unit: "coin".into(),
                acquired_date: Some("2026-01-10".into()),
                effective_date: None,
                acquired_cost: cost.map(usd),
                acquired_from: None,
                storage_location: Some("Safe".into()),
                notes: String::new(),
                insured: None,
                attrs: [
                    ("year".to_string(), "1921".to_string()),
                    ("cert_number".to_string(), "123".to_string()),
                ]
                .into_iter()
                .collect(),
                pricing: Pricing::Manual,
                review_every_days: None,
            },
            NOW,
        )
        .unwrap()
    }

    fn value(v: &Vault, id: &str, minor: i64, qty: i64) {
        crate::valuations::record_valuation(
            v,
            &crate::valuations::NewValuation {
                asset_id: id.into(),
                quote_id: None,
                value: usd(minor),
                quantity_at_time: Decimal::from(qty),
                basis: crate::valuations::Basis::EstimatedResale,
                provenance: crate::valuations::Provenance::Manual,
                inputs: serde_json::json!({}),
                asof: "2026-02-01".into(),
            },
            NOW,
        )
        .unwrap();
    }

    #[test]
    fn splitting_preserves_partial_known_cost_without_claiming_completeness() {
        let (_d, v) = setup();
        let id = holding(&v, "Partial cost", 1, Some(10001));
        events::record(
            &v,
            &NewEvent {
                asset_id: id.clone(),
                event_type: EventType::Add,
                effective_date: TODAY.into(),
                quantity_delta: Decimal::ONE,
                amount_minor: None,
                currency: None,
                note: String::new(),
            },
            NOW,
        )
        .unwrap();
        let child = split(&v, &id, Decimal::ONE, "Half", TODAY, NOW).unwrap();
        let original = assets::get(&v, &id).unwrap();
        let child = assets::get(&v, &child).unwrap();
        assert_eq!(
            original.acquired_amount_minor.unwrap() + child.acquired_amount_minor.unwrap(),
            10001
        );
        assert!(!original.cost_complete && !child.cost_complete);
    }

    #[test]
    fn odd_cents_survive_splits_repeated_splits_and_cache_rebuilds() {
        for minor in [10001, 10003] {
            let (_d, v) = setup();
            let original = holding(&v, "Odd cents", 4, Some(minor));
            value(&v, &original, minor, 4);
            let historical = crate::valuations::portfolio_total_as_of(
                &v,
                "2026-03-01",
                &Currency::new("USD").unwrap(),
            )
            .unwrap()
            .total;
            let first = split(&v, &original, Decimal::from(2), "Half", TODAY, NOW).unwrap();
            let second = split(&v, &original, Decimal::ONE, "Quarter", TODAY, NOW).unwrap();
            let mut costs = 0;
            let mut values = 0;
            for id in [&original, &first, &second] {
                events::rebuild_cost_in(v.conn(), id).unwrap();
                crate::valuations::refresh_current_value_in(v.conn(), id, NOW).unwrap();
                let asset = assets::get(&v, id).unwrap();
                costs += asset.acquired_amount_minor.unwrap();
                values += asset.current_amount_minor.unwrap();
            }
            assert_eq!(costs, minor);
            assert_eq!(values, minor);
            assert_eq!(
                crate::valuations::portfolio_total_as_of(
                    &v,
                    "2026-03-01",
                    &Currency::new("USD").unwrap()
                )
                .unwrap()
                .total,
                historical
            );
        }
    }

    #[test]
    fn a_split_divides_cost_and_value_and_leaves_every_total_unchanged() {
        let (_d, v) = setup();
        let roll = holding(&v, "Morgan dollars", 20, Some(60_000));
        value(&v, &roll, 80_000, 20);
        let usd_c = Currency::new("USD").unwrap();
        let before_march =
            crate::valuations::portfolio_total_as_of(&v, "2026-03-01", &usd_c).unwrap().total;
        let before_today =
            crate::valuations::portfolio_total_as_of(&v, TODAY, &usd_c).unwrap().total;

        let three =
            split(&v, &roll, Decimal::from(3), "Morgan dollars — best three", TODAY, NOW)
                .unwrap();
        let r = assets::get(&v, &roll).unwrap();
        let n = assets::get(&v, &three).unwrap();
        assert_eq!((r.quantity.as_str(), n.quantity.as_str()), ("17", "3"));
        assert_eq!(
            (r.acquired_amount_minor, n.acquired_amount_minor),
            (Some(51_000), Some(9_000))
        );
        assert_eq!(n.current_amount_minor, Some(12_000));
        assert_eq!(n.acquired_date.as_deref(), Some("2026-01-10"), "first acquired then");
        assert!(!n.attrs.contains_key("cert_number"), "identifiers stay with the original");
        assert_eq!(n.attrs.get("year").map(String::as_str), Some("1921"));
        assert_eq!(r.status, "active", "a split is not a sale");

        let after_march =
            crate::valuations::portfolio_total_as_of(&v, "2026-03-01", &usd_c).unwrap().total;
        let after_today =
            crate::valuations::portfolio_total_as_of(&v, TODAY, &usd_c).unwrap().total;
        assert_eq!(after_march, before_march, "no double counting before the split");
        assert_eq!(after_today, before_today);
        assert!(
            split(&v, &roll, Decimal::from(17), "all", TODAY, NOW).is_err(),
            "not the whole holding"
        );
    }

    #[test]
    fn a_set_sums_its_members_and_a_purchase_divides_to_the_cent() {
        let (_d, v) = setup();
        let a = holding(&v, "A", 1, None);
        let b = holding(&v, "B", 1, None);
        let c = holding(&v, "C", 1, None);
        value(&v, &a, 1_000, 1);
        value(&v, &b, 2_000, 1);
        value(&v, &c, 7_000, 1);
        let set = create_set(&v, "1921 set", Some(5), "", NOW).unwrap();
        assert_eq!(add_members(&v, &set, &[a.clone(), b.clone(), c.clone()], NOW).unwrap(), 3);
        let summary = &sets(&v, &Currency::new("USD").unwrap(), TODAY).unwrap()[0];
        assert_eq!(
            (summary.members, summary.value_minor, summary.target_count),
            (3, 10_000, Some(5))
        );

        let equal =
            allocate_cost(&v, &[a.clone(), b.clone(), c.clone()], &usd(1_000), false, NOW)
                .unwrap();
        assert_eq!(equal.iter().map(|s| s.1).sum::<i64>(), 1_000, "adds up exactly");
        assert_eq!(equal.iter().map(|s| s.1).collect::<Vec<_>>(), vec![334, 333, 333]);
        let weighted = allocate_cost(&v, &[a.clone(), b, c], &usd(5_000), true, NOW).unwrap();
        assert_eq!(weighted.iter().map(|s| s.1).collect::<Vec<_>>(), vec![500, 1_000, 3_500]);
        assert_eq!(assets::get(&v, &a).unwrap().acquired_amount_minor, Some(500));

        delete_set(&v, &set).unwrap();
        assert!(assets::get(&v, &a).is_ok(), "members are untouched");
    }
}
