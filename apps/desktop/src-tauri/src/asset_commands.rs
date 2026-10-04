//! Asset records, photos and CSV.
//!
//! Validation happens here, in the backend, on every write path. The forms
//! call the same validators for live feedback, but a form is a convenience,
//! not a control.

use std::collections::BTreeMap;

use am_core::{collectibles, parse_address, Chain, Currency, Decimal, Money};
use am_storage::assets::{self, AssetEdit, AssetRecord, NewAsset, Pricing};
use am_storage::pricing::MarketSpec;
use am_storage::valuations::{self, Basis, NewValuation, Provenance};
use am_storage::vault::Vault;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime, State};

use crate::ipc::{
    bad_input, base_currency, currency_or, format_money, now, parse_money_opt, storage, today,
    IpcResult,
};
use crate::paths::vault_root;
use crate::session::{IpcError, Session, SessionError};

/// Bound on a single attribute value and on how many a record may carry, so
/// a pasted document cannot bloat a row.
const MAX_ATTR_LEN: usize = 2_000;
const MAX_ATTRS: usize = 60;

fn other(message: String) -> IpcError {
    IpcError { kind: "error".into(), message }
}

fn asset_err(e: assets::AssetError) -> IpcError {
    match e {
        assets::AssetError::Sqlite(e) => IpcError::from(storage(e)),
        other => bad_input(other.to_string()),
    }
}

// ------------------------------------------------------------ views

/// An asset as the frontend sees it: the stored record, plus the figures
/// derived from it, formatted here so no float ever touches money.
#[derive(Serialize)]
pub struct AssetView {
    #[serde(flatten)]
    pub record: AssetRecord,
    pub current_display: Option<String>,
    pub acquired_display: Option<String>,
    pub insured_display: Option<String>,
    pub sold_display: Option<String>,
    /// Current value minus cost, when both are known in the same currency
    /// and the cost covers the whole holding. Absent otherwise — never
    /// computed against an assumed zero, or against the cost of only some of
    /// the units.
    pub gain_minor: Option<String>,
    pub gain_display: Option<String>,
    pub next_review: Option<String>,
    pub review_due: bool,
}

pub fn view(record: AssetRecord, today: &str) -> AssetView {
    let current = record.current_amount_minor.zip(record.current_currency.clone());
    let cost = record.acquired_amount_minor.zip(record.acquired_currency.clone());
    let gain = match (&current, &cost) {
        (Some((value, vc)), Some((cost, cc)))
            if vc == cc && record.status == "active" && record.cost_complete =>
        {
            Currency::new(vc).ok().and_then(|c| {
                Money::new(*value, c.clone()).checked_sub(&Money::new(*cost, c)).ok()
            })
        }
        _ => None,
    };

    let next_review = record.review_every_days.and_then(|days| {
        let from = record.value_asof.clone().unwrap_or_else(|| {
            record.created_at[..10.min(record.created_at.len())].to_string()
        });
        am_storage::summary::add_days(&from, days)
    });
    let review_due = record.status == "active"
        && next_review.as_deref().map(|d| d <= today).unwrap_or(false);

    AssetView {
        current_display: format_money(
            record.current_amount_minor,
            record.current_currency.as_deref(),
        ),
        acquired_display: format_money(
            record.acquired_amount_minor,
            record.acquired_currency.as_deref(),
        ),
        insured_display: format_money(
            record.insured_amount_minor,
            record.insured_currency.as_deref(),
        ),
        sold_display: format_money(record.sold_amount_minor, record.sold_currency.as_deref()),
        gain_minor: gain.as_ref().map(|g| g.amount_minor.to_string()),
        gain_display: gain.as_ref().map(Money::format),
        next_review,
        review_due,
        record,
    }
}

#[tauri::command]
pub fn asset_types(session: State<'_, Session>) -> IpcResult<Vec<assets::AssetType>> {
    session.touch();
    session.with_vault(|vault| assets::types(vault).map_err(storage)).map_err(IpcError::from)
}

#[tauri::command]
pub fn list_assets(session: State<'_, Session>) -> IpcResult<Vec<AssetView>> {
    session.touch();
    let today = today();
    session
        .with_vault(|vault| {
            let records = assets::list(vault).map_err(storage)?;
            Ok(records.into_iter().map(|r| view(r, &today)).collect())
        })
        .map_err(IpcError::from)
}

/// IDs matching a search, best first. The frontend already holds the
/// records, so a search returns only which ones and in what order.
#[tauri::command]
pub fn search_assets(session: State<'_, Session>, query: String) -> IpcResult<Vec<String>> {
    session.touch();
    session
        .with_vault(|vault| assets::search(vault, &query).map_err(storage))
        .map_err(IpcError::from)
}

#[derive(Serialize)]
pub struct EventView {
    pub event_id: String,
    pub event_type: String,
    pub effective_date: String,
    pub quantity_delta: String,
    pub amount_display: Option<String>,
    pub note: String,
    pub recorded_at: String,
}

#[derive(Serialize)]
pub struct ValuationView {
    pub valuation_id: String,
    pub asof: String,
    pub amount_minor: String,
    pub amount: String,
    pub currency: String,
    pub basis: String,
    pub provenance: String,
    pub quantity_at_time: String,
    pub note: Option<String>,
    /// For market valuations: the unit price used and when the source set it.
    pub unit_price: Option<String>,
    pub quote_source: Option<String>,
    pub quote_asof: Option<String>,
}

#[derive(Serialize)]
pub struct PhotoRef {
    pub object_id: String,
    pub media_type: String,
    pub is_primary: bool,
}

#[derive(Serialize)]
pub struct MarketView {
    /// "Gold spot", "BTC".
    pub label: String,
    pub instrument_id: String,
    pub unit_price: Option<String>,
    pub unit: String,
    pub currency: Option<String>,
    pub source: Option<String>,
    pub source_asof: Option<String>,
    /// Fine metal content of the current holding, for metals.
    pub fine_troy_oz: Option<String>,
}

#[derive(Serialize)]
pub struct AssetDetail {
    pub asset: AssetView,
    pub events: Vec<EventView>,
    pub valuations: Vec<ValuationView>,
    pub photos: Vec<PhotoRef>,
    pub market: Option<MarketView>,
}

fn photos_of(vault: &Vault, asset_id: &str) -> Result<Vec<PhotoRef>, SessionError> {
    let mut stmt = vault
        .conn()
        .prepare(
            "SELECT m.object_id, o.media_type, m.is_primary
             FROM asset_media m
             JOIN objects o ON o.object_id = m.object_id
             WHERE m.asset_id = ?1 AND o.gc_state = 'live'
             ORDER BY m.is_primary DESC, m.sort_order, m.created_at",
        )
        .map_err(storage)?;
    let rows = stmt
        .query_map([asset_id], |r| {
            Ok(PhotoRef {
                object_id: r.get(0)?,
                media_type: r.get(1)?,
                is_primary: r.get::<_, i64>(2)? == 1,
            })
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(rows)
}

fn valuations_of(vault: &Vault, asset_id: &str) -> Result<Vec<ValuationView>, SessionError> {
    let mut stmt = vault
        .conn()
        .prepare(
            "SELECT v.valuation_id, v.asof, v.amount_minor, v.currency, v.basis, v.provenance,
                    v.quantity_at_time, v.inputs, q.unit_quote, q.source, q.source_asof
             FROM valuations v LEFT JOIN quotes q ON q.quote_id = v.quote_id
             WHERE v.asset_id = ?1
             ORDER BY v.asof DESC, v.recorded_at DESC, v.rowid DESC",
        )
        .map_err(storage)?;
    let rows = stmt
        .query_map([asset_id], |r| {
            let minor: i64 = r.get(2)?;
            let code: String = r.get(3)?;
            let inputs: String = r.get(7)?;
            let note = serde_json::from_str::<serde_json::Value>(&inputs)
                .ok()
                .and_then(|v| v.get("note").and_then(|n| n.as_str()).map(str::to_string))
                .filter(|n| !n.is_empty());
            Ok(ValuationView {
                valuation_id: r.get(0)?,
                asof: r.get(1)?,
                amount_minor: minor.to_string(),
                amount: format_money(Some(minor), Some(&code)).unwrap_or_default(),
                currency: code,
                basis: r.get(4)?,
                provenance: r.get(5)?,
                quantity_at_time: r.get(6)?,
                note,
                unit_price: r.get(8)?,
                quote_source: r.get(9)?,
                quote_asof: r.get(10)?,
            })
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(rows)
}

fn market_of(vault: &Vault, record: &AssetRecord) -> Option<MarketView> {
    let spec = MarketSpec::from_attrs(&record.attrs).ok().flatten()?;
    let instrument_id = spec.instrument_id();
    let quote: Option<(String, String, String, String)> = vault
        .conn()
        .query_row(
            "SELECT unit_quote, currency, source, source_asof FROM quotes
             WHERE instrument_id = ?1
             ORDER BY source_asof DESC, fetched_at DESC, rowid DESC LIMIT 1",
            [&instrument_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .ok();

    let (label, unit, fine) = match &spec {
        MarketSpec::Metal { metal, weight_per_item, unit, basis, purity, .. } => {
            let quantity = am_core::parse_decimal(&record.quantity).unwrap_or_default();
            let fine = am_core::MetalHolding {
                quantity,
                weight_per_item: *weight_per_item,
                weight_unit: *unit,
                weight_basis: *basis,
                purity: *purity,
            }
            .fine_weight(am_core::WeightUnit::TroyOunce)
            .ok()
            .map(|f| f.round_dp(4).normalize().to_string());
            (format!("{metal} spot"), "troy oz".to_string(), fine)
        }
        MarketSpec::Coin { identity } => (identity.display_label(), "coin".to_string(), None),
    };

    Some(MarketView {
        label,
        instrument_id,
        unit_price: quote.as_ref().map(|q| q.0.clone()),
        unit,
        currency: quote.as_ref().map(|q| q.1.clone()),
        source: quote.as_ref().map(|q| q.2.clone()),
        source_asof: quote.map(|q| q.3),
        fine_troy_oz: fine,
    })
}

#[tauri::command]
pub fn get_asset(session: State<'_, Session>, asset_id: String) -> IpcResult<AssetDetail> {
    session.touch();
    let today = today();
    session
        .with_vault(|vault| {
            let record = assets::get(vault, &asset_id).map_err(storage)?;
            let events = am_storage::events::history(vault, &asset_id)
                .map_err(storage)?
                .into_iter()
                .rev()
                .map(|e| EventView {
                    event_id: e.event_id,
                    event_type: e.event_type.as_str().to_string(),
                    effective_date: e.effective_date,
                    quantity_delta: e.quantity_delta.normalize().to_string(),
                    amount_display: format_money(e.amount_minor, e.currency.as_deref()),
                    note: e.note,
                    recorded_at: e.recorded_at,
                })
                .collect();
            let market = market_of(vault, &record);
            Ok(AssetDetail {
                events,
                valuations: valuations_of(vault, &asset_id)?,
                photos: photos_of(vault, &asset_id)?,
                market,
                asset: view(record, &today),
            })
        })
        .map_err(IpcError::from)
}

// ------------------------------------------------------------ writes

/// The asset form, for both create and edit.
///
/// Amounts are major units as typed ("1299.50"). Each amount has its own
/// currency: `acquired_currency` and `insured_currency` when given, otherwise
/// — on edit — the currency already stored with that amount, and only then
/// `currency` or the vault's base currency. Changing the base currency must
/// never relabel a stored USD 1,000 as EUR 1,000 because a form was saved.
#[derive(Deserialize, Default)]
pub struct AssetForm {
    pub type_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    /// Create only. Afterwards, quantity changes are events.
    #[serde(default)]
    pub quantity: Option<String>,
    #[serde(default)]
    pub quantity_unit: Option<String>,
    #[serde(default)]
    pub acquired_date: Option<String>,
    #[serde(default)]
    pub acquired_price: Option<String>,
    #[serde(default)]
    pub acquired_from: Option<String>,
    #[serde(default)]
    pub storage_location: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub insured_value: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub acquired_currency: Option<String>,
    #[serde(default)]
    pub insured_currency: Option<String>,
    #[serde(default)]
    pub attrs: BTreeMap<String, String>,
    /// "manual" or "market". Create only; afterwards use `set_pricing`.
    #[serde(default)]
    pub pricing: Option<String>,
    #[serde(default)]
    pub review_every_days: Option<i64>,
    /// Create only: an opening valuation, recorded as a manual value.
    #[serde(default)]
    pub current_value: Option<String>,
    /// Edit only: the cost given covers everything held, even if unchanged.
    #[serde(default)]
    pub cost_covers_holding: bool,
}

/// Validate and clean attributes for a type.
///
/// Returns the cleaned map and a name derived from it, for types that can
/// describe themselves (a comic's title and issue, a coin's year and
/// denomination).
fn prepare_attrs(
    type_id: &str,
    attrs: &BTreeMap<String, String>,
) -> IpcResult<(BTreeMap<String, String>, Option<String>)> {
    if attrs.len() > MAX_ATTRS {
        return Err(bad_input("too many fields on one record"));
    }
    let mut cleaned: BTreeMap<String, String> = BTreeMap::new();
    for (key, value) in attrs {
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            continue;
        }
        if value.chars().count() > MAX_ATTR_LEN || key.len() > 64 {
            return Err(bad_input(format!("{key} is too long")));
        }
        cleaned.insert(key.to_string(), value.to_string());
    }

    // A watch-only address is validated before it is stored, which refuses
    // anything resembling a seed phrase or private key: a vault must never
    // hold something that can move funds.
    if let Some(address) = cleaned.get("watch_address").cloned() {
        let chain_name =
            cleaned.get("watch_chain").cloned().unwrap_or_else(|| "bitcoin".into());
        let chain = Chain::parse(&chain_name)
            .ok_or_else(|| bad_input(format!("unknown chain: {chain_name}")))?;
        let parsed =
            parse_address(chain, &address, "").map_err(|e| bad_input(e.to_string()))?;
        cleaned.insert("watch_address".into(), parsed.address);
        cleaned.insert("watch_chain".into(), chain.as_str().to_string());
    }

    let mut derived_name = None;
    if collectibles::collectible_type(type_id).is_some() {
        cleaned =
            collectibles::validate(type_id, &cleaned).map_err(|e| bad_input(e.to_string()))?;
        derived_name = Some(collectibles::describe(type_id, &cleaned));
    }

    MarketSpec::from_attrs(&cleaned).map_err(|e| bad_input(e.to_string()))?;
    Ok((cleaned, derived_name))
}

/// The currency for one amount on the form: the one named for that field,
/// else the one already stored with it, else the form's general currency.
fn field_currency(
    explicit: &Option<String>,
    stored: Option<&str>,
    general: &Currency,
) -> IpcResult<Currency> {
    match explicit.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        Some(code) => Currency::new(code).map_err(|e| bad_input(e.to_string())),
        None => match stored {
            Some(code) => Currency::new(code).map_err(|e| bad_input(e.to_string())),
            None => Ok(general.clone()),
        },
    }
}

fn pick_name(typed: &Option<String>, derived: Option<String>) -> IpcResult<String> {
    match typed.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => Ok(name.to_string()),
        _ => derived
            .filter(|d| !d.trim().is_empty())
            .ok_or_else(|| bad_input("give this asset a name")),
    }
}

/// Record an owner-entered value, switching the asset to manual pricing.
fn record_manual_value(
    vault: &Vault,
    asset_id: &str,
    value: Money,
    note: &str,
    now: &str,
) -> Result<(), SessionError> {
    let today = &today();
    let quantity =
        am_storage::events::quantity_as_of(vault, asset_id, Some(today)).map_err(storage)?;
    valuations::record_valuation(
        vault,
        &NewValuation {
            asset_id: asset_id.to_string(),
            quote_id: None,
            value,
            quantity_at_time: quantity,
            basis: Basis::EstimatedResale,
            provenance: Provenance::Manual,
            inputs: serde_json::json!({ "note": note }),
            asof: today.to_string(),
        },
        now,
    )
    .map_err(storage)?;
    assets::set_pricing(vault, asset_id, Pricing::Manual, now).map_err(storage)?;
    Ok(())
}

#[tauri::command]
pub fn validate_asset(session: State<'_, Session>, form: AssetForm) -> IpcResult<String> {
    session.touch();
    let (_, derived) = prepare_attrs(&form.type_id, &form.attrs)?;
    pick_name(&form.name, derived)
}

#[tauri::command]
pub fn create_asset(session: State<'_, Session>, form: AssetForm) -> IpcResult<String> {
    session.touch();
    let timestamp = now();
    let (attrs, derived) = prepare_attrs(&form.type_id, &form.attrs)?;
    let name = pick_name(&form.name, derived)?;
    let quantity = match form.quantity.as_deref().map(str::trim) {
        None | Some("") => Decimal::ONE,
        Some(q) => assets::parse_quantity(q).map_err(asset_err)?,
    };
    let pricing = match form.pricing.as_deref() {
        Some("market") => Pricing::Market,
        _ => Pricing::Manual,
    };
    if pricing == Pricing::Market
        && MarketSpec::from_attrs(&attrs).map_err(|e| bad_input(e.to_string()))?.is_none()
    {
        return Err(bad_input("market pricing needs a metal or a coin to follow"));
    }

    session
        .with_vault(|vault| {
            let currency = currency_or(&form.currency, &base_currency(vault))
                .map_err(|e| storage(e.message))?;
            let cost_currency = field_currency(&form.acquired_currency, None, &currency)
                .map_err(|e| storage(e.message))?;
            let insured_currency = field_currency(&form.insured_currency, None, &currency)
                .map_err(|e| storage(e.message))?;
            let cost = parse_money_opt(&form.acquired_price, &cost_currency)
                .map_err(|e| storage(e.message))?;
            let insured = parse_money_opt(&form.insured_value, &insured_currency)
                .map_err(|e| storage(e.message))?;
            let opening = parse_money_opt(&form.current_value, &currency)
                .map_err(|e| storage(e.message))?;

            let asset_id = assets::create(
                vault,
                &NewAsset {
                    type_id: form.type_id.clone(),
                    name: name.clone(),
                    quantity,
                    quantity_unit: form.quantity_unit.clone().unwrap_or_default(),
                    acquired_date: form.acquired_date.clone(),
                    acquired_cost: cost,
                    acquired_from: form.acquired_from.clone(),
                    storage_location: form.storage_location.clone(),
                    notes: form.notes.clone().unwrap_or_default(),
                    insured,
                    attrs: attrs.clone(),
                    pricing,
                    review_every_days: form.review_every_days,
                },
                &timestamp,
            )
            .map_err(storage)?;

            if let Some(value) = opening {
                record_manual_value(vault, &asset_id, value, "opening value", &timestamp)?;
            } else if pricing == Pricing::Market {
                am_storage::pricing::revalue_asset(vault, &asset_id, &timestamp)
                    .map_err(storage)?;
            }
            Ok(asset_id)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn update_asset(
    session: State<'_, Session>,
    asset_id: String,
    form: AssetForm,
) -> IpcResult<()> {
    session.touch();
    let timestamp = now();
    let (attrs, derived) = prepare_attrs(&form.type_id, &form.attrs)?;
    let name = pick_name(&form.name, derived)?;

    session
        .with_vault(|vault| {
            let current = assets::get(vault, &asset_id).map_err(storage)?;
            let currency = currency_or(&form.currency, &base_currency(vault))
                .map_err(|e| storage(e.message))?;
            let cost_currency = field_currency(
                &form.acquired_currency,
                current.acquired_currency.as_deref(),
                &currency,
            )
            .map_err(|e| storage(e.message))?;
            let insured_currency = field_currency(
                &form.insured_currency,
                current.insured_currency.as_deref(),
                &currency,
            )
            .map_err(|e| storage(e.message))?;
            let cost = parse_money_opt(&form.acquired_price, &cost_currency)
                .map_err(|e| storage(e.message))?;
            let insured = parse_money_opt(&form.insured_value, &insured_currency)
                .map_err(|e| storage(e.message))?;

            assets::update(
                vault,
                &asset_id,
                &AssetEdit {
                    type_id: form.type_id.clone(),
                    name: name.clone(),
                    status: form.status.clone().unwrap_or(current.status.clone()),
                    quantity_unit: form
                        .quantity_unit
                        .clone()
                        .unwrap_or(current.quantity_unit.clone()),
                    acquired_date: form.acquired_date.clone(),
                    acquired_cost: cost,
                    acquired_from: form.acquired_from.clone(),
                    storage_location: form.storage_location.clone(),
                    notes: form.notes.clone().unwrap_or_default(),
                    insured,
                    attrs: attrs.clone(),
                    review_every_days: form.review_every_days,
                    cost_covers_holding: form.cost_covers_holding,
                },
                &timestamp,
            )
            .map_err(storage)?;

            // Changed weight or purity on a market holding changes its value.
            if current.pricing == "market" {
                am_storage::pricing::revalue_asset(vault, &asset_id, &timestamp)
                    .map_err(storage)?;
            }
            Ok(())
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn delete_asset<R: Runtime>(
    app: AppHandle<R>,
    session: State<'_, Session>,
    asset_id: String,
) -> IpcResult<()> {
    session.touch();
    let root = vault_root(&app).map_err(other)?;
    session
        .with_vault(|vault| assets::delete(vault, &root, &asset_id).map_err(storage))
        .map_err(IpcError::from)
}

/// Follow the market, or stop following it.
///
/// Switching to market revalues immediately, so the figure shown changes as
/// soon as the choice is made rather than at the next refresh.
#[tauri::command]
pub fn set_pricing(
    session: State<'_, Session>,
    asset_id: String,
    pricing: String,
) -> IpcResult<()> {
    session.touch();
    let timestamp = now();
    let pricing = match pricing.as_str() {
        "market" => Pricing::Market,
        "manual" => Pricing::Manual,
        other => return Err(bad_input(format!("unknown pricing: {other}"))),
    };
    session
        .with_vault(|vault| {
            if pricing == Pricing::Market {
                let record = assets::get(vault, &asset_id).map_err(storage)?;
                if MarketSpec::from_attrs(&record.attrs).map_err(storage)?.is_none() {
                    return Err(storage("this asset has no metal or coin to follow"));
                }
            }
            assets::set_pricing(vault, &asset_id, pricing, &timestamp).map_err(storage)?;
            if pricing == Pricing::Market {
                am_storage::pricing::revalue_asset(vault, &asset_id, &timestamp)
                    .map_err(storage)?;
            }
            Ok(())
        })
        .map_err(IpcError::from)
}

// ------------------------------------------------------------ photos

#[derive(Serialize)]
pub struct ImportedPhoto {
    pub object_id: String,
    pub media_type: String,
    /// True when identical bytes were already stored and nothing was written.
    pub deduplicated: bool,
}

/// Import a photo from a path the user chose in a file dialog.
///
/// The path comes from the frontend, so it is read as the user — this imports
/// a file they picked, it does not grant the vault arbitrary filesystem reach
/// beyond what the user already has.
#[tauri::command]
pub fn import_photo<R: Runtime>(
    app: AppHandle<R>,
    session: State<'_, Session>,
    asset_id: String,
    path: String,
) -> IpcResult<ImportedPhoto> {
    session.touch();
    let root = vault_root(&app).map_err(other)?;
    let timestamp = now();

    let size = std::fs::metadata(&path)
        .map_err(|e| IpcError { kind: "unreadable_file".into(), message: e.to_string() })?
        .len();
    if size > am_storage::objects::MAX_IMPORT_BYTES {
        return Err(IpcError {
            kind: "too_large".into(),
            message: format!(
                "that file is larger than the {} MiB limit",
                am_storage::objects::MAX_IMPORT_BYTES / 1024 / 1024
            ),
        });
    }

    // Read before taking the vault lock: file I/O should not hold it.
    let bytes = std::fs::read(&path)
        .map_err(|e| IpcError { kind: "unreadable_file".into(), message: e.to_string() })?;

    let stored = session
        .with_vault(|vault| {
            let stored = am_storage::objects::import_object(vault, &root, &bytes, &timestamp)
                .map_err(storage)?;
            let already: i64 = vault
                .conn()
                .query_row(
                    "SELECT count(*) FROM asset_media WHERE asset_id = ?1 AND object_id = ?2",
                    [&asset_id, &stored.object_id],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if already == 0 {
                am_storage::objects::attach_to_asset(
                    vault,
                    &asset_id,
                    &stored.object_id,
                    &timestamp,
                )
                .map_err(storage)?;
            }

            // Thumbnails are generated inline. When this moves to a
            // background worker it must be cancellable on lock, or a late
            // result could write into a vault that has since closed.
            let _ = am_storage::thumbs::generate_variants(
                vault,
                &root,
                &stored.object_id,
                &timestamp,
            );
            Ok(stored)
        })
        .map_err(IpcError::from)?;

    Ok(ImportedPhoto {
        object_id: stored.object_id,
        media_type: stored.media_type,
        deduplicated: stored.deduplicated,
    })
}

#[tauri::command]
pub fn list_photos(session: State<'_, Session>, asset_id: String) -> IpcResult<Vec<PhotoRef>> {
    session.touch();
    session.with_vault(|vault| photos_of(vault, &asset_id)).map_err(IpcError::from)
}

/// Detach a photo and sweep it if nothing else references it.
#[tauri::command]
pub fn remove_photo<R: Runtime>(
    app: AppHandle<R>,
    session: State<'_, Session>,
    asset_id: String,
    object_id: String,
) -> IpcResult<()> {
    session.touch();
    let root = vault_root(&app).map_err(other)?;
    session
        .with_vault(|vault| {
            am_storage::objects::detach_from_asset(vault, &asset_id, &object_id)
                .map_err(storage)?;
            am_storage::objects::sweep_deleted(vault, &root).map_err(storage)?;
            Ok(())
        })
        .map_err(IpcError::from)
}

/// Save a decrypted copy of a photo or document to a path the owner chose.
///
/// The one way to get an attachment back out — a receipt kept for a claim is
/// no use if it can only be looked at as a label. Only an object attached to
/// the named asset can be exported, and never into the vault folder. The
/// caller has already warned that the copy is plaintext.
#[tauri::command]
pub fn export_attachment<R: Runtime>(
    app: AppHandle<R>,
    session: State<'_, Session>,
    asset_id: String,
    object_id: String,
    path: String,
) -> IpcResult<()> {
    session.touch();
    let root = vault_root(&app).map_err(other)?;
    let target = std::path::PathBuf::from(&path);
    if target.starts_with(&root) {
        return Err(bad_input("choose a location outside the vault folder"));
    }
    let bytes = session
        .with_vault(|vault| {
            let attached: i64 = vault
                .conn()
                .query_row(
                    "SELECT count(*) FROM asset_media WHERE asset_id = ?1 AND object_id = ?2",
                    [&asset_id, &object_id],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if attached == 0 {
                return Err(storage("that file is not attached to this asset"));
            }
            am_storage::objects::load_object(vault, &root, &object_id).map_err(storage)
        })
        .map_err(IpcError::from)?;
    // Written outside the vault lock: a slow disk must not hold the session.
    std::fs::write(&target, bytes.as_slice())
        .map_err(|e| IpcError { kind: "unwritable_file".into(), message: e.to_string() })
}

#[tauri::command]
pub fn set_primary_photo(
    session: State<'_, Session>,
    asset_id: String,
    object_id: String,
) -> IpcResult<()> {
    session.touch();
    session
        .with_vault(|vault| {
            assets::set_primary_photo(vault, &asset_id, &object_id).map_err(storage)
        })
        .map_err(IpcError::from)
}

// ------------------------------------------------------------ CSV

#[derive(Serialize)]
pub struct ExportedCsv {
    pub csv: String,
    pub row_count: usize,
    /// Shown verbatim in the UI before the file is saved. An export leaves the
    /// vault's protection entirely, and users will not infer that.
    pub warning: String,
}

#[tauri::command]
pub fn export_csv(session: State<'_, Session>) -> IpcResult<ExportedCsv> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            let result = am_storage::csv::export_assets(vault, &timestamp).map_err(storage)?;
            Ok(ExportedCsv {
                csv: result.csv,
                row_count: result.row_count,
                warning: "This file is not encrypted and is not a backup. It excludes \
                          photos, documents and valuation history. Anyone who can read \
                          the file can read your catalog."
                    .to_string(),
            })
        })
        .map_err(IpcError::from)
}

#[derive(Serialize)]
pub struct ImportSummary {
    pub creates: usize,
    pub updates: usize,
    pub errors: Vec<String>,
}

/// Preview or apply a CSV import.
///
/// Always call with `apply = false` first: the preview reports every row-level
/// problem at once, so the spreadsheet can be fixed in one pass.
#[tauri::command]
pub fn import_csv(
    session: State<'_, Session>,
    contents: String,
    apply: bool,
) -> IpcResult<ImportSummary> {
    session.touch();
    let timestamp = now();
    let mode = if apply {
        am_storage::csv::ImportMode::Apply
    } else {
        am_storage::csv::ImportMode::Preview
    };

    session
        .with_vault(|vault| {
            let preview = am_storage::csv::import_assets(vault, &contents, mode, &timestamp)
                .map_err(storage)?;
            Ok(ImportSummary {
                creates: preview.creates,
                updates: preview.updates,
                errors: preview.errors,
            })
        })
        .map_err(IpcError::from)
}

/// Read a user-chosen text file.
///
/// Deliberately narrow rather than granting the filesystem plugin: this reads
/// one path the user picked in a dialog, with a size bound, and nothing else.
/// A general fs permission would widen the attack surface for no benefit.
#[tauri::command]
pub fn read_text_file(path: String) -> IpcResult<String> {
    let meta = std::fs::metadata(&path)
        .map_err(|e| IpcError { kind: "unreadable_file".into(), message: e.to_string() })?;
    if meta.len() as usize > am_storage::csv::MAX_FILE_BYTES {
        return Err(IpcError {
            kind: "too_large".into(),
            message: "file is too large to import".into(),
        });
    }
    std::fs::read_to_string(&path)
        .map_err(|e| IpcError { kind: "unreadable_file".into(), message: e.to_string() })
}

/// Write a user-chosen text file.
///
/// The caller has already warned that the contents leave the vault's
/// protection; this only performs the write.
#[tauri::command]
pub fn write_text_file(path: String, contents: String) -> IpcResult<()> {
    std::fs::write(&path, contents)
        .map_err(|e| IpcError { kind: "unwritable_file".into(), message: e.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn each_amount_keeps_its_own_currency_unless_told_otherwise() {
        let eur = Currency::new("EUR").unwrap();
        let code = |explicit: Option<&str>, stored: Option<&str>| {
            field_currency(&explicit.map(str::to_string), stored, &eur)
                .unwrap()
                .code()
                .to_string()
        };
        assert_eq!(code(None, Some("USD")), "USD", "stored beats the form's general currency");
        assert_eq!(code(Some("JPY"), Some("USD")), "JPY", "an explicit choice beats both");
        assert_eq!(code(Some("  "), Some("USD")), "USD", "blank is not a choice");
        assert_eq!(code(None, None), "EUR", "a new amount takes the general currency");
        assert!(field_currency(&Some("dollars".into()), None, &eur).is_err());
    }

    #[test]
    fn collectibles_are_validated_and_named_from_their_fields() {
        let (cleaned, name) = prepare_attrs(
            "comic",
            &attrs(&[
                ("title", " Amazing Fantasy "),
                ("issue", "15"),
                ("grader", "cgc"),
                ("grade", "9.8"),
                ("variant", ""),
            ]),
        )
        .unwrap();
        assert_eq!(name.as_deref(), Some("Amazing Fantasy #15 — CGC 9.8"));
        assert!(!cleaned.contains_key("variant"), "empty fields are dropped");

        let err = prepare_attrs(
            "comic",
            &attrs(&[("title", "X"), ("issue", "1"), ("grader", "psa"), ("grade", "65")]),
        )
        .unwrap_err();
        assert!(err.message.contains("PSA"));
    }

    #[test]
    fn a_seed_phrase_is_never_stored_as_a_watch_address() {
        let seed = "abandon abandon abandon abandon abandon abandon abandon abandon \
                    abandon abandon abandon about";
        let err =
            prepare_attrs("crypto", &attrs(&[("coin_id", "bitcoin"), ("watch_address", seed)]))
                .unwrap_err();
        assert!(
            err.message.to_lowercase().contains("seed") || err.message.contains("recovery"),
            "{}",
            err.message
        );
    }

    #[test]
    fn a_valid_watch_address_is_normalized() {
        let (cleaned, _) = prepare_attrs(
            "crypto",
            &attrs(&[
                ("coin_id", "bitcoin"),
                ("watch_address", " 1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa "),
            ]),
        )
        .unwrap();
        assert_eq!(cleaned["watch_address"], "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa");
        assert_eq!(cleaned["watch_chain"], "bitcoin");
    }

    #[test]
    fn metal_attributes_are_checked_before_saving() {
        assert!(prepare_attrs(
            "gold_bullion",
            &attrs(&[("metal", "XAU"), ("weight_per_item", "1"), ("purity", "0.9999")])
        )
        .is_ok());
        let err = prepare_attrs(
            "gold_bullion",
            &attrs(&[("metal", "XAU"), ("weight_per_item", "1"), ("purity", "9999")]),
        )
        .unwrap_err();
        assert!(err.message.contains("purity"));
    }

    #[test]
    fn a_name_is_required_unless_one_can_be_derived() {
        assert!(pick_name(&None, None).is_err());
        assert!(pick_name(&Some("  ".into()), None).is_err());
        assert_eq!(pick_name(&None, Some("Derived".into())).unwrap(), "Derived");
        assert_eq!(pick_name(&Some("Typed".into()), Some("Derived".into())).unwrap(), "Typed");
    }

    #[test]
    fn gain_needs_both_value_and_cost_in_one_currency() {
        let base = blank_record;

        let mut r = base();
        r.current_amount_minor = Some(15_000);
        r.current_currency = Some("USD".into());
        assert!(view(r.clone(), "2026-09-22").gain_minor.is_none(), "unknown cost: no gain");

        r.acquired_amount_minor = Some(10_000);
        r.acquired_currency = Some("USD".into());
        assert_eq!(view(r.clone(), "2026-09-22").gain_minor.as_deref(), Some("5000"));

        let mut partial = r.clone();
        partial.cost_complete = false;
        assert!(
            view(partial, "2026-09-22").gain_minor.is_none(),
            "never against a cost that covers only part of the holding"
        );

        r.acquired_currency = Some("EUR".into());
        assert!(view(r, "2026-09-22").gain_minor.is_none(), "never across currencies");
    }

    #[test]
    fn reviews_come_due_from_the_last_valuation() {
        let mut r = AssetRecord {
            review_every_days: Some(30),
            value_asof: Some("2026-08-01".into()),
            ..blank_record()
        };
        assert_eq!(view(r.clone(), "2026-08-20").next_review.as_deref(), Some("2026-08-31"));
        assert!(!view(r.clone(), "2026-08-20").review_due);
        assert!(view(r.clone(), "2026-09-01").review_due);
        r.status = "sold".into();
        assert!(!view(r, "2026-09-01").review_due, "nothing to review once sold");
    }

    fn blank_record() -> AssetRecord {
        AssetRecord {
            asset_id: "a".into(),
            type_id: "generic".into(),
            type_label: "Generic".into(),
            archetype: "unique".into(),
            category: "other".into(),
            name: "x".into(),
            status: "active".into(),
            quantity: "1".into(),
            quantity_unit: "item".into(),
            acquired_date: None,
            acquired_amount_minor: None,
            acquired_currency: None,
            cost_complete: true,
            acquired_from: None,
            storage_location: None,
            notes: String::new(),
            current_amount_minor: None,
            current_currency: None,
            value_source: None,
            value_asof: None,
            insured_amount_minor: None,
            insured_currency: None,
            sold_date: None,
            sold_amount_minor: None,
            sold_currency: None,
            attrs: BTreeMap::new(),
            pricing: "manual".into(),
            review_every_days: None,
            primary_photo: None,
            photo_count: 0,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }
}
