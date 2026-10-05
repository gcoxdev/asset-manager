//! Insurance report.
//!
//! The one output here that exists for someone else to read: an insurer
//! handling a claim. That shapes the design — it needs provenance for each
//! figure, identifying details (serials, cert numbers), photographs, and a
//! clear statement of what the numbers are and are not.
//!
//! # Plaintext by nature
//!
//! A report is meant to be sent, so it cannot be encrypted and still be
//! useful. That makes it the single easiest way to leak an entire catalogue,
//! and the caller warns before writing one. Storage locations are optional
//! for the same reason: an insurer rarely needs to know which drawer.

use std::collections::BTreeMap;

use am_core::{Currency, Money};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::ipc::{base_currency, format_money, now, storage, IpcResult};
use crate::session::{IpcError, Session};

#[derive(Serialize)]
pub struct ReportItem {
    pub name: String,
    /// "active" or "lost". Lost items appear only when asked for, for a claim.
    pub status: String,
    /// When a lost item was lost.
    pub lost_on: Option<String>,
    pub type_label: String,
    pub category: String,
    pub quantity: String,
    pub quantity_unit: String,
    pub storage_location: Option<String>,
    pub acquired_date: Option<String>,
    pub acquired: Option<String>,
    pub acquired_from: Option<String>,
    pub current: Option<String>,
    /// Where the current figure came from, so a valuation is not presented as
    /// authoritative when someone typed it in.
    pub value_source: Option<String>,
    pub value_asof: Option<String>,
    pub insured: Option<String>,
    /// Identifying details — serials, cert numbers, grades — as label/value
    /// pairs in a stable order.
    pub details: Vec<(String, String)>,
    pub notes: Option<String>,
    /// Object IDs, resolved to embedded images by the caller.
    pub photo_ids: Vec<String>,
    /// Receipts, appraisals and other documents on file, by title — so an
    /// assessor knows what evidence exists to ask for. Not their contents.
    pub documents: Vec<ReportDocument>,
    /// With `compare_bases`: the latest value on each basis.
    pub values_by_basis: Vec<BasisValue>,
}

#[derive(Serialize)]
pub struct BasisValue {
    pub basis: String,
    pub value: String,
    pub asof: String,
}

#[derive(Serialize)]
pub struct ReportDocument {
    pub kind: String,
    pub title: Option<String>,
    pub date: Option<String>,
}

#[derive(Serialize)]
pub struct ReportCategory {
    pub category: String,
    pub count: usize,
    pub total: String,
}

#[derive(Serialize)]
pub struct InsuranceReport {
    pub generated_at: String,
    /// The date values and quantities are as of, when not today.
    pub as_of: Option<String>,
    pub items: Vec<ReportItem>,
    pub categories: Vec<ReportCategory>,
    pub total: String,
    pub insured_total: String,
    pub valued: usize,
    /// Items with no valuation. Reported rather than omitted: an insurer
    /// should see that the total is partial.
    pub unvalued: usize,
    /// Items included because they are marked lost.
    pub lost: usize,
    /// Other currencies converted into the totals, with the rate used.
    pub converted: Vec<am_storage::fx::Applied>,
    pub currency: String,
    pub warning: String,
}

/// What goes into a report beyond the item itself.
///
/// Leaving an option out is always the private choice — the same choice the
/// report screen starts from. Storage locations and notes appear only when
/// asked for by name: a list of where valuables are kept is the most
/// damaging thing a leaked report could contain.
#[derive(Deserialize)]
pub struct ReportOptions {
    #[serde(default)]
    pub include_locations: bool,
    #[serde(default)]
    pub include_notes: bool,
    #[serde(default = "yes")]
    pub include_photos: bool,
    /// Include items marked lost, with their last value before the loss —
    /// what a claim is made from. Off unless asked for.
    #[serde(default)]
    pub include_lost: bool,
    /// List each item's documents by title and date. Off unless asked for:
    /// a title can say more than intended ("Safe deposit box 114 receipt").
    #[serde(default)]
    pub include_documents: bool,
    /// Exactly these assets, whatever their status — a claim covers the
    /// items it is about and discloses nothing else. Absent: every held
    /// asset (plus lost ones with `include_lost`).
    #[serde(default)]
    pub asset_ids: Option<Vec<String>>,
    /// Quantities and values as they stood on this date — the day before a
    /// loss, for a claim — rather than today.
    #[serde(default)]
    pub as_of: Option<String>,
    /// Each item's latest value on every basis — resale, replacement,
    /// insured, melt — side by side, as of the same date.
    #[serde(default)]
    pub compare_bases: bool,
}

impl Default for ReportOptions {
    fn default() -> Self {
        Self {
            include_locations: false,
            include_notes: false,
            include_photos: true,
            include_lost: false,
            include_documents: false,
            asset_ids: None,
            as_of: None,
            compare_bases: false,
        }
    }
}

fn yes() -> bool {
    true
}

/// Attribute keys worth showing an insurer, with their labels, in order.
/// Anything else a record carries is shown after these, labelled from its key.
const DETAIL_ORDER: &[(&str, &str)] = &[
    ("serial_number", "Serial"),
    ("cert_number", "Certificate"),
    ("grader", "Grader"),
    ("grade", "Grade"),
    ("manufacturer", "Manufacturer"),
    ("model", "Model"),
    ("caliber", "Caliber"),
    ("vin", "VIN"),
    ("hull_id", "Hull ID"),
    ("registration", "Registration"),
    ("parcel_number", "Parcel number"),
    ("make", "Make"),
    ("brand", "Brand"),
    ("publisher", "Publisher"),
    ("year", "Year"),
    ("set", "Set"),
    ("card_number", "Card number"),
    ("denomination", "Denomination"),
    ("mintmark", "Mint mark"),
    ("metal", "Metal"),
    ("weight_per_item", "Weight each"),
    ("weight_unit", "Weight unit"),
    ("purity", "Purity"),
    ("coin_id", "Coin"),
];

/// Keys that describe how to price or look something up, not the item.
const INTERNAL: &[&str] = &[
    "preset",
    "weight_basis",
    "premium_pct",
    "watch_address",
    "watch_chain",
    "chain",
    "contract",
];

fn label_for(key: &str) -> String {
    if let Some((_, label)) = DETAIL_ORDER.iter().find(|(k, _)| *k == key) {
        return (*label).to_string();
    }
    let words = key.replace('_', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn details(attrs: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = DETAIL_ORDER
        .iter()
        .filter_map(|(k, label)| attrs.get(*k).map(|v| ((*label).to_string(), v.clone())))
        .collect();
    for (key, value) in attrs {
        if DETAIL_ORDER.iter().any(|(k, _)| k == key) || INTERNAL.contains(&key.as_str()) {
            continue;
        }
        out.push((label_for(key), value.clone()));
    }
    out
}

/// Build an insurance report over active holdings.
#[tauri::command]
pub fn insurance_report(
    session: State<'_, Session>,
    options: Option<ReportOptions>,
) -> IpcResult<InsuranceReport> {
    session.touch();
    let options = options.unwrap_or_default();
    let generated_at = now();

    session
        .with_vault(|vault| {
            let currency: Currency = base_currency(vault);
            let code = currency.code().to_string();
            // What is held — plus, when asked, what was lost: a claim needs
            // those items and their values from before the loss. Sold and
            // retired items are never included.
            let as_of = match options.as_of.as_deref().map(str::trim) {
                None | Some("") => None,
                Some(d) => Some(
                    am_storage::events::normalize_date(d)
                        .map_err(|e| storage(e.to_string()))?,
                ),
            };
            let all = am_storage::assets::list(vault).map_err(storage)?;
            let mut records: Vec<_> = match &options.asset_ids {
                Some(ids) => {
                    if ids.is_empty() || ids.len() > 5_000 {
                        return Err(storage("choose between 1 and 5,000 items"));
                    }
                    all.into_iter().filter(|r| ids.contains(&r.asset_id)).collect()
                }
                None => all
                    .into_iter()
                    .filter(|r| {
                        r.status == "active" || (options.include_lost && r.status == "lost")
                    })
                    .collect(),
            };
            if let Some(date) = &as_of {
                for r in &mut records {
                    as_of_figures(vault, r, date)?;
                }
                // Not yet acquired on that date: nothing to claim.
                records.retain(|r| r.quantity != "0");
            }
            let mut lost = 0usize;
            let mut converted: Vec<am_storage::fx::Applied> = Vec::new();

            let mut items = Vec::new();
            let mut total = Money::zero(currency.clone());
            let mut insured_total = Money::zero(currency.clone());
            let mut categories: Vec<(String, usize, Money)> = Vec::new();
            let (mut valued, mut unvalued) = (0usize, 0usize);

            let mut sorted = records;
            sorted.sort_by(|a, b| {
                a.category
                    .cmp(&b.category)
                    .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });

            for r in sorted {
                let slot = match categories.iter().position(|c| c.0 == r.category) {
                    Some(i) => i,
                    None => {
                        categories.push((r.category.clone(), 0, Money::zero(currency.clone())));
                        categories.len() - 1
                    }
                };
                categories[slot].1 += 1;

                // Another currency counts at the rate in effect on the
                // report's date, and the report says which rate — a silently
                // converted total would be wrong in a way an insurer could
                // not see. Without a rate it is listed but not totalled.
                let rate_date = as_of.clone().unwrap_or_else(crate::ipc::today);
                let mut in_base = |amount: i64, c: &str| -> Result<Option<Money>, crate::session::SessionError> {
                    if c == code {
                        return Ok(Some(Money::new(amount, currency.clone())));
                    }
                    let foreign = Money::new(amount, Currency::new(c).map_err(storage)?);
                    Ok(match am_storage::fx::convert(vault.conn(), &foreign, &currency, &rate_date)
                        .map_err(storage)?
                    {
                        Some((m, applied)) => {
                            if !converted.iter().any(|a: &am_storage::fx::Applied| a.from_currency == applied.from_currency) {
                                converted.push(applied);
                            }
                            Some(m)
                        }
                        None => None,
                    })
                };
                match (r.current_amount_minor, r.current_currency.as_deref()) {
                    (Some(amount), Some(c)) => match in_base(amount, c)? {
                        Some(m) => {
                            total = total.checked_add(&m).map_err(storage)?;
                            categories[slot].2 =
                                categories[slot].2.checked_add(&m).map_err(storage)?;
                            valued += 1;
                        }
                        None => unvalued += 1,
                    },
                    _ => unvalued += 1,
                }
                if let (Some(amount), Some(c)) =
                    (r.insured_amount_minor, r.insured_currency.as_deref())
                {
                    if let Some(m) = in_base(amount, c)? {
                        insured_total = insured_total.checked_add(&m).map_err(storage)?;
                    }
                }

                let photo_ids = if options.include_photos {
                    let mut photos = vault
                        .conn()
                        .prepare(
                            "SELECT m.object_id FROM asset_media m
                             JOIN objects o ON o.object_id = m.object_id
                             WHERE m.asset_id = ?1 AND o.gc_state = 'live'
                               AND o.media_type LIKE 'image/%' AND m.doc_kind = 'photo'
                             ORDER BY m.is_primary DESC, m.sort_order LIMIT 4",
                        )
                        .map_err(storage)?;
                    let ids = photos
                        .query_map([&r.asset_id], |row| row.get(0))
                        .map_err(storage)?
                        .collect::<Result<Vec<String>, _>>()
                        .map_err(storage)?;
                    ids
                } else {
                    Vec::new()
                };

                let documents = if options.include_documents {
                    let mut stmt = vault
                        .conn()
                        .prepare(
                            "SELECT m.doc_kind, m.title, m.doc_date FROM asset_media m
                             JOIN objects o ON o.object_id = m.object_id
                             WHERE m.asset_id = ?1 AND o.gc_state = 'live'
                               AND m.doc_kind <> 'photo'
                             ORDER BY coalesce(m.doc_date, m.created_at)",
                        )
                        .map_err(storage)?;
                    let docs = stmt
                        .query_map([&r.asset_id], |row| {
                            Ok(ReportDocument {
                                kind: row.get(0)?,
                                title: row.get(1)?,
                                date: row.get(2)?,
                            })
                        })
                        .map_err(storage)?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(storage)?;
                    docs
                } else {
                    Vec::new()
                };
                let lost_on = if r.status == "lost" {
                    lost += 1;
                    am_storage::lifecycle::history(vault, &r.asset_id)
                        .map_err(storage)?
                        .into_iter()
                        .rev()
                        .find(|e| e.status == "lost")
                        .map(|e| e.effective_date)
                } else {
                    None
                };
                let values_by_basis = if options.compare_bases {
                    values_by_basis(vault, &r, as_of.as_deref())?
                } else {
                    Vec::new()
                };
                items.push(ReportItem {
                    values_by_basis,
                    status: r.status.clone(),
                    lost_on,
                    details: details(&r.attrs),
                    current: format_money(
                        r.current_amount_minor,
                        r.current_currency.as_deref(),
                    ),
                    acquired: format_money(
                        r.acquired_amount_minor,
                        r.acquired_currency.as_deref(),
                    ),
                    insured: format_money(
                        r.insured_amount_minor,
                        r.insured_currency.as_deref(),
                    ),
                    storage_location: if options.include_locations {
                        r.storage_location
                    } else {
                        None
                    },
                    notes: if options.include_notes && !r.notes.is_empty() {
                        Some(r.notes)
                    } else {
                        None
                    },
                    name: r.name,
                    type_label: r.type_label,
                    category: r.category,
                    quantity: r.quantity,
                    quantity_unit: r.quantity_unit,
                    acquired_date: r.acquired_date,
                    acquired_from: r.acquired_from,
                    value_source: r.value_source,
                    value_asof: r.value_asof,
                    photo_ids,
                    documents,
                });
            }

            Ok(InsuranceReport {
                as_of: as_of.clone(),
                generated_at: generated_at.clone(),
                items,
                categories: categories
                    .into_iter()
                    .map(|(category, count, sum)| ReportCategory {
                        category,
                        count,
                        total: sum.format(),
                    })
                    .collect(),
                total: total.format(),
                insured_total: insured_total.format(),
                valued,
                unvalued,
                lost,
                converted,
                currency: code.clone(),
                warning: "This report is not encrypted. It lists what you own and what it \
                          is worth — treat the file as you would the items themselves."
                    .to_string(),
            })
        })
        .map_err(IpcError::from)
}

#[derive(Serialize)]
pub struct ClaimFiles {
    pub folder: String,
    pub files: usize,
}

/// A file name that is safe on every platform, from an asset and attachment.
fn claim_file_name(index: usize, asset: &str, title: &str, extension: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_alphanumeric() || " -_.,()&'".contains(c) { c } else { ' ' })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(80)
            .collect()
    };
    format!("{:02} {} - {}.{}", index, clean(asset), clean(title), extension)
}

/// Decrypt the chosen assets' documents — and photos, if asked — into a new
/// folder inside `directory`, for sending with a claim. Only those assets'
/// files are written; nothing else from the catalog leaves the vault. The
/// caller has warned that the copies are not encrypted.
#[tauri::command]
pub fn export_claim_files<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    session: State<'_, Session>,
    asset_ids: Vec<String>,
    directory: String,
    include_photos: bool,
) -> IpcResult<ClaimFiles> {
    session.touch();
    let root = crate::paths::vault_root(&app)
        .map_err(|m| IpcError { kind: "error".into(), message: m })?;
    let parent = std::path::PathBuf::from(&directory);
    if !parent.is_dir() {
        return Err(crate::ipc::bad_input("choose an existing folder"));
    }
    if parent.starts_with(&root) {
        return Err(crate::ipc::bad_input("choose a folder outside the vault itself"));
    }
    if asset_ids.is_empty() || asset_ids.len() > 5_000 {
        return Err(crate::ipc::bad_input("choose between 1 and 5,000 items"));
    }

    // A new folder, never into one with files already in it.
    let base = format!("Claim files {}", crate::ipc::today());
    let mut folder = parent.join(&base);
    let mut n = 2;
    while folder.exists() {
        folder = parent.join(format!("{base} ({n})"));
        n += 1;
    }

    session
        .with_vault(|vault| {
            std::fs::create_dir_all(&folder).map_err(storage)?;
            let mut written = 0usize;
            for asset_id in &asset_ids {
                let record = am_storage::assets::get(vault, asset_id).map_err(storage)?;
                let mut stmt = vault
                    .conn()
                    .prepare(
                        "SELECT m.object_id, o.media_type, m.doc_kind, m.title FROM asset_media m
                         JOIN objects o ON o.object_id = m.object_id
                         WHERE m.asset_id = ?1 AND o.gc_state = 'live'
                           AND (m.doc_kind <> 'photo' OR ?2)
                         ORDER BY m.doc_kind = 'photo', coalesce(m.doc_date, m.created_at)",
                    )
                    .map_err(storage)?;
                let attachments = stmt
                    .query_map(rusqlite::params![asset_id, include_photos], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, Option<String>>(3)?,
                        ))
                    })
                    .map_err(storage)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(storage)?;
                for (object_id, media_type, kind, title) in attachments {
                    let bytes = am_storage::objects::load_object(vault, &root, &object_id)
                        .map_err(storage)?;
                    let extension = match media_type.as_str() {
                        "application/pdf" => "pdf",
                        "image/png" => "png",
                        "image/webp" => "webp",
                        "image/heic" => "heic",
                        _ => "jpg",
                    };
                    written += 1;
                    let name = claim_file_name(
                        written,
                        &record.name,
                        title.as_deref().unwrap_or(&kind),
                        extension,
                    );
                    std::fs::write(folder.join(name), bytes.as_slice()).map_err(storage)?;
                }
            }
            Ok(written)
        })
        .map(|files| ClaimFiles { folder: folder.display().to_string(), files })
        .map_err(|e| {
            // Leave no half-written folder behind.
            let _ = std::fs::remove_dir_all(&folder);
            IpcError::from(e)
        })
}

/// Rewrite a record's quantity and value to how they stood on `date`: the
/// quantity replayed from the log, the valuation in effect then, scaled to
/// that quantity.
fn as_of_figures(
    vault: &am_storage::vault::Vault,
    r: &mut am_storage::assets::AssetRecord,
    date: &str,
) -> Result<(), crate::session::SessionError> {
    let quantity =
        am_storage::events::quantity_as_of(vault, &r.asset_id, Some(date)).map_err(storage)?;
    r.quantity = quantity.normalize().to_string();
    match am_storage::valuations::valuation_as_of(vault, &r.asset_id, date).map_err(storage)? {
        Some(v) if !quantity.is_zero() => {
            let scaled = am_storage::valuations::scale_to_quantity(
                &v.value,
                v.quantity_at_time,
                quantity,
            )
            .map_err(storage)?;
            r.current_amount_minor = Some(scaled.amount_minor);
            r.current_currency = Some(scaled.currency.code().to_string());
            r.value_source = Some(v.provenance.as_str().to_string());
            r.value_asof = Some(v.asof);
        }
        _ => {
            r.current_amount_minor = None;
            r.current_currency = None;
            r.value_source = None;
            r.value_asof = None;
        }
    }
    Ok(())
}

/// The latest value on each basis, as of a date (today if none), scaled to
/// the quantity then held.
fn values_by_basis(
    vault: &am_storage::vault::Vault,
    r: &am_storage::assets::AssetRecord,
    as_of: Option<&str>,
) -> Result<Vec<BasisValue>, crate::session::SessionError> {
    let date = as_of.map(str::to_string).unwrap_or_else(crate::ipc::today);
    let quantity =
        am_storage::events::quantity_as_of(vault, &r.asset_id, Some(&date)).map_err(storage)?;
    let mut stmt = vault
        .conn()
        .prepare(
            "SELECT basis, amount_minor, currency, quantity_at_time, asof FROM valuations
             WHERE asset_id = ?1 AND voided_at IS NULL AND asof <= ?2
             ORDER BY asof DESC, recorded_at DESC, rowid DESC",
        )
        .map_err(storage)?;
    let rows = stmt
        .query_map([&r.asset_id, &date], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let mut out: Vec<BasisValue> = Vec::new();
    for (basis, minor, code, at_time, asof) in rows {
        if out.iter().any(|b| b.basis == basis) {
            continue;
        }
        let currency = Currency::new(&code).map_err(storage)?;
        let at_time = am_core::parse_decimal(&at_time).map_err(storage)?;
        let value = if quantity.is_zero() {
            Money::new(minor, currency)
        } else {
            am_storage::valuations::scale_to_quantity(
                &Money::new(minor, currency),
                at_time,
                quantity,
            )
            .map_err(storage)?
        };
        out.push(BasisValue { basis, value: value.format(), asof });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifying_details_come_first_and_internals_are_hidden() {
        let attrs: BTreeMap<String, String> = [
            ("zodiac", "Leo"),
            ("grade", "9.8"),
            ("cert_number", "0012345"),
            ("premium_pct", "4"),
            ("watch_address", "bc1q..."),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

        let out = details(&attrs);
        assert_eq!(out[0], ("Certificate".to_string(), "0012345".to_string()));
        assert_eq!(out[1], ("Grade".to_string(), "9.8".to_string()));
        assert_eq!(
            out[2],
            ("Zodiac".to_string(), "Leo".to_string()),
            "unknown keys still shown"
        );
        assert_eq!(out.len(), 3, "pricing internals are not item details");
    }

    #[test]
    fn leaving_an_option_out_is_the_private_choice() {
        let omitted = ReportOptions::default();
        assert!(!omitted.include_locations && !omitted.include_notes);
        let partial: ReportOptions =
            serde_json::from_str(r#"{"include_photos": false}"#).unwrap();
        assert!(!partial.include_locations, "a missing field must not reveal locations");
        assert!(!partial.include_notes);
        let empty: ReportOptions = serde_json::from_str("{}").unwrap();
        assert!(!empty.include_locations && empty.include_photos);
    }

    #[test]
    fn claim_file_names_are_safe_everywhere() {
        assert_eq!(
            claim_file_name(3, "Rolex: Submariner / 116610", "Receipt <2021>", "pdf"),
            "03 Rolex Submariner 116610 - Receipt 2021.pdf"
        );
    }

    #[test]
    fn money_formatting_respects_the_currency() {
        assert_eq!(format_money(Some(129_950), Some("USD")).as_deref(), Some("1299.50 USD"));
        // JPY has no minor units; 1000 is a thousand yen, not ten.
        assert_eq!(format_money(Some(1000), Some("JPY")).as_deref(), Some("1000 JPY"));
        // Never a zero, which would read as "worthless" rather than "unpriced".
        assert_eq!(format_money(None, Some("USD")), None);
    }
}
