//! IPC for precious metals.
//!
//! Three things the plan asks for that are visible here:
//!
//! - **Melt and market shown separately.** Melt is the metal content at spot;
//!   market adds a premium. A numismatic coin can be worth far more than its
//!   melt, and conflating the two would misprice a collection in whichever
//!   direction the premium happens to lie.
//! - **Manual spot override**, which never charges the API quota and always
//!   works.
//! - **Staleness on every derived figure**, judged by the source's timestamp.

use am_core::{
    parse_decimal, valuation::WeightBasis, valuation::WeightUnit, Currency, Metal, PRESETS,
};
use am_storage::spot::{self, SpotOrigin, SpotReading};
use am_storage::vault::VaultError;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::session::{IpcError, Session, SessionError};

type IpcResult<T> = Result<T, IpcError>;

/// The provider whose quota is being tracked. One provider for now; when a
/// second arrives this becomes a parameter rather than a constant.
const METALS_PROVIDER: &str = "metals.dev";

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn bad_input(message: impl Into<String>) -> IpcError {
    IpcError { kind: "invalid_input".into(), message: message.into() }
}

fn storage(e: impl std::fmt::Display) -> SessionError {
    SessionError::Vault(VaultError::Other(e.to_string()))
}

#[derive(Serialize)]
pub struct PresetInfo {
    pub id: String,
    pub label: String,
    pub metal: String,
    pub metal_name: String,
    pub weight: String,
    pub unit: String,
    /// "gross" or "fine". Surfaced because it is the field most often
    /// misunderstood, and a form that hides it invites a silent 8% error.
    pub basis: String,
    pub purity: String,
}

fn unit_name(unit: WeightUnit) -> &'static str {
    match unit {
        WeightUnit::TroyOunce => "troy_oz",
        WeightUnit::Gram => "gram",
        WeightUnit::Pennyweight => "pennyweight",
        WeightUnit::Ounce => "ounce",
    }
}

fn parse_unit(name: &str) -> Option<WeightUnit> {
    Some(match name {
        "troy_oz" => WeightUnit::TroyOunce,
        "gram" => WeightUnit::Gram,
        "pennyweight" => WeightUnit::Pennyweight,
        "ounce" => WeightUnit::Ounce,
        _ => return None,
    })
}

#[tauri::command]
pub fn bullion_presets() -> Vec<PresetInfo> {
    PRESETS
        .iter()
        .map(|p| PresetInfo {
            id: p.id.to_string(),
            label: p.label.to_string(),
            metal: p.metal.code().to_string(),
            metal_name: p.metal.display_name().to_string(),
            weight: p.weight.to_string(),
            unit: unit_name(p.unit).to_string(),
            basis: match p.basis {
                WeightBasis::Gross => "gross",
                WeightBasis::Fine => "fine",
            }
            .to_string(),
            purity: p.purity.to_string(),
        })
        .collect()
}

#[derive(Serialize)]
pub struct SpotView {
    pub metal: String,
    pub metal_name: String,
    pub price_per_troy_oz: Option<String>,
    pub currency: Option<String>,
    pub source: Option<String>,
    pub source_asof: Option<String>,
    pub freshness: Option<String>,
    pub needs_caveat: bool,
    pub age_hours: Option<i64>,
}

#[tauri::command]
pub fn spot_prices(session: State<'_, Session>) -> IpcResult<Vec<SpotView>> {
    session.touch();
    let timestamp = now();

    session
        .with_vault(|vault| {
            let mut out = Vec::new();
            for metal in Metal::ALL {
                let latest = spot::latest_spot(vault, *metal, &timestamp).map_err(storage)?;
                out.push(match latest {
                    Some(price) => SpotView {
                        metal: price.metal,
                        metal_name: metal.display_name().to_string(),
                        price_per_troy_oz: Some(price.price_per_troy_oz),
                        currency: Some(price.currency),
                        source: Some(price.source),
                        source_asof: Some(price.source_asof),
                        freshness: Some(price.freshness),
                        needs_caveat: price.needs_caveat,
                        age_hours: Some(price.age_hours),
                    },
                    None => SpotView {
                        metal: metal.code().to_string(),
                        metal_name: metal.display_name().to_string(),
                        price_per_troy_oz: None,
                        currency: None,
                        source: None,
                        source_asof: None,
                        freshness: None,
                        // No price is not a stale price; the UI shows "not
                        // set" rather than a warning about age.
                        needs_caveat: false,
                        age_hours: None,
                    },
                });
            }
            Ok(out)
        })
        .map_err(IpcError::from)
}

/// Enter a spot price by hand.
///
/// Never charges the API quota, and works with the provider down or the
/// allowance spent — a manual price is a first-class input.
#[tauri::command]
pub fn set_spot_price(
    session: State<'_, Session>,
    metal: String,
    price: String,
    currency: Option<String>,
) -> IpcResult<()> {
    session.touch();
    let timestamp = now();

    let metal =
        Metal::parse(&metal).ok_or_else(|| bad_input(format!("unknown metal: {metal}")))?;
    let price = spot::parse_spot_input(&price).map_err(|e| bad_input(e.to_string()))?;
    let currency = Currency::new(&currency.unwrap_or_else(|| "USD".into()))
        .map_err(|e| bad_input(e.to_string()))?;

    session
        .with_vault(|vault| {
            spot::record_spot(
                vault,
                SpotReading {
                    metal,
                    price_per_troy_oz: price,
                    currency: &currency,
                    source: "manual",
                    // A hand-entered price is current as of now, by
                    // definition — the person is looking at it.
                    source_asof: &timestamp,
                    origin: SpotOrigin::Manual,
                },
                &timestamp,
            )
            .map_err(storage)?;
            Ok(())
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn metals_quota(session: State<'_, Session>) -> IpcResult<am_storage::spot::QuotaStatus> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            spot::quota_status(vault, METALS_PROVIDER, &timestamp).map_err(storage)
        })
        .map_err(IpcError::from)
}

#[derive(Deserialize)]
pub struct MetalValuationRequest {
    pub metal: String,
    /// Number of items.
    pub quantity: String,
    /// Weight of one item.
    pub weight_per_item: String,
    pub unit: String,
    /// "gross" or "fine".
    pub basis: String,
    pub purity: String,
    /// Premium over melt as a percentage, e.g. "5" for 5%.
    pub premium_pct: Option<String>,
}

#[derive(Serialize)]
pub struct MetalValuation {
    pub fine_troy_oz: String,
    /// Metal content at spot, no premium.
    pub melt: String,
    /// Melt plus premium. Equal to melt when no premium is given.
    pub market: String,
    pub premium_amount: String,
    pub spot_used: String,
    pub currency: String,
    pub source_asof: Option<String>,
    pub freshness: Option<String>,
    /// True when the spot price behind these figures is old enough to warrant
    /// saying so next to them.
    pub needs_caveat: bool,
}

/// Value a metal holding at the stored spot price.
///
/// Returns melt and market separately: a graded coin's premium can dwarf its
/// metal content, and showing one number would hide which is which.
#[tauri::command]
pub fn value_metal_holding(
    session: State<'_, Session>,
    request: MetalValuationRequest,
) -> IpcResult<MetalValuation> {
    session.touch();
    let timestamp = now();

    let metal = Metal::parse(&request.metal)
        .ok_or_else(|| bad_input(format!("unknown metal: {}", request.metal)))?;
    let unit = parse_unit(&request.unit)
        .ok_or_else(|| bad_input(format!("unknown weight unit: {}", request.unit)))?;
    let basis = match request.basis.as_str() {
        "gross" => WeightBasis::Gross,
        "fine" => WeightBasis::Fine,
        other => return Err(bad_input(format!("basis must be gross or fine, got {other}"))),
    };

    let quantity =
        parse_decimal(&request.quantity).map_err(|_| bad_input("quantity is not a number"))?;
    let weight = parse_decimal(&request.weight_per_item)
        .map_err(|_| bad_input("weight is not a number"))?;
    let purity =
        parse_decimal(&request.purity).map_err(|_| bad_input("purity is not a number"))?;

    // Entered as a percentage because that is how premiums are quoted;
    // converted to a fraction for the arithmetic.
    let premium_pct = match request.premium_pct.as_deref() {
        None | Some("") => am_core::Decimal::ZERO,
        Some(text) => {
            parse_decimal(text).map_err(|_| bad_input("premium is not a number"))?
                / am_core::Decimal::from(100)
        }
    };

    let holding = am_core::MetalHolding {
        quantity,
        weight_per_item: weight,
        weight_unit: unit,
        weight_basis: basis,
        purity,
    };

    session
        .with_vault(|vault| {
            let spot_price = spot::latest_spot(vault, metal, &timestamp)
                .map_err(storage)?
                .ok_or_else(|| {
                    storage(format!(
                        "no spot price for {} — enter one to value this holding",
                        metal.display_name()
                    ))
                })?;

            let spot_value = parse_decimal(&spot_price.price_per_troy_oz)
                .map_err(|_| storage("stored spot price is malformed"))?;
            let currency =
                Currency::new(&spot_price.currency).map_err(|e| storage(e.to_string()))?;

            let fine = holding
                .fine_weight(WeightUnit::TroyOunce)
                .map_err(|e| storage(e.to_string()))?;
            let melt = holding
                .melt_value(spot_value, currency.clone())
                .map_err(|e| storage(e.to_string()))?;
            let market = holding
                .market_value(spot_value, premium_pct, currency.clone())
                .map_err(|e| storage(e.to_string()))?;
            let premium_amount =
                market.checked_sub(&melt).map_err(|e| storage(e.to_string()))?;

            Ok(MetalValuation {
                fine_troy_oz: fine.round_dp(4).normalize().to_string(),
                melt: melt.format(),
                market: market.format(),
                premium_amount: premium_amount.format(),
                spot_used: spot_price.price_per_troy_oz,
                currency: spot_price.currency,
                source_asof: Some(spot_price.source_asof),
                freshness: Some(spot_price.freshness),
                needs_caveat: spot_price.needs_caveat,
            })
        })
        .map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_exposed_with_its_basis() {
        let presets = bullion_presets();
        assert_eq!(presets.len(), PRESETS.len());

        // Basis must reach the UI: hiding it is how the gross/fine error gets
        // made.
        let eagle = presets.iter().find(|p| p.id == "age").unwrap();
        assert_eq!(eagle.basis, "gross");
        assert_eq!(eagle.purity, "0.9167");

        let maple = presets.iter().find(|p| p.id == "maple_gold").unwrap();
        assert_eq!(maple.basis, "fine");
    }

    #[test]
    fn weight_units_round_trip_through_their_names() {
        for unit in [
            WeightUnit::TroyOunce,
            WeightUnit::Gram,
            WeightUnit::Pennyweight,
            WeightUnit::Ounce,
        ] {
            assert_eq!(parse_unit(unit_name(unit)), Some(unit));
        }
        assert_eq!(parse_unit("kilograms"), None);
    }

    #[test]
    fn presets_name_a_real_metal() {
        for preset in bullion_presets() {
            assert!(
                Metal::parse(&preset.metal).is_some(),
                "preset {} has an unparseable metal code",
                preset.id
            );
        }
    }
}

// ---------------------------------------------------------------- provider

use crate::metals_provider;

#[derive(Serialize)]
pub struct ProviderStatus {
    pub configured: bool,
    pub quota: am_storage::spot::QuotaStatus,
}

#[tauri::command]
pub fn metals_provider_status(session: State<'_, Session>) -> IpcResult<ProviderStatus> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            Ok(ProviderStatus {
                configured: metals_provider::has_api_key(),
                quota: spot::quota_status(vault, METALS_PROVIDER, &timestamp)
                    .map_err(storage)?,
            })
        })
        .map_err(IpcError::from)
}

/// Store the metals.dev API key in the OS keyring.
///
/// Requires an unlocked vault — not because the key is stored there, but
/// because configuring a provider is an owner action, and the lock is what
/// establishes the owner is present.
#[tauri::command]
pub fn set_metals_api_key(session: State<'_, Session>, key: String) -> IpcResult<()> {
    session.touch();
    session.with_vault(|_| Ok(())).map_err(IpcError::from)?;

    if key.trim().is_empty() {
        metals_provider::clear_api_key()
            .map_err(|e| IpcError { kind: "keyring".into(), message: e.to_string() })
    } else {
        metals_provider::store_api_key(&key)
            .map_err(|e| IpcError { kind: "keyring".into(), message: e.to_string() })
    }
}

#[derive(Serialize)]
pub struct RefreshResult {
    pub updated: Vec<String>,
    pub source_asof: Option<String>,
    pub quota: am_storage::spot::QuotaStatus,
}

/// Fetch fresh spot prices.
///
/// `automatic` distinguishes a background poll from a user asking. Both count
/// against the monthly budget, but automatic polling stops at its smaller
/// allowance so a background task cannot spend the requests the owner needs
/// on demand.
#[tauri::command]
pub fn refresh_spot_prices(
    session: State<'_, Session>,
    automatic: Option<bool>,
) -> IpcResult<RefreshResult> {
    session.touch();
    let timestamp = now();
    let automatic = automatic.unwrap_or(false);

    // Budget check before the network call, so a refused request costs
    // nothing.
    let allowed = session
        .with_vault(|vault| {
            let status =
                spot::quota_status(vault, METALS_PROVIDER, &timestamp).map_err(storage)?;
            Ok(if automatic {
                status.may_poll_automatically
            } else {
                status.may_refresh_manually
            })
        })
        .map_err(IpcError::from)?;

    if !allowed {
        return Err(IpcError {
            kind: "quota_exhausted".into(),
            message: if automatic {
                "automatic updates have used their share of this month's budget".into()
            } else {
                "this month's request budget is spent — enter a price by hand instead".into()
            },
        });
    }

    // The network call happens outside the vault lock: a slow response must
    // not hold the database for its duration.
    let prices = metals_provider::fetch_spot_prices("USD").map_err(|e| IpcError {
        kind: match e {
            metals_provider::ProviderError::NoKey => "no_api_key",
            metals_provider::ProviderError::QuotaExhausted => "quota_exhausted",
            _ => "provider_error",
        }
        .into(),
        message: e.to_string(),
    })?;

    let source_asof = prices.first().map(|p| p.source_asof.clone());

    session
        .with_vault(|vault| {
            let mut updated = Vec::new();
            for price in &prices {
                let Some(metal) = Metal::parse(&price.metal) else { continue };
                let value = parse_decimal(&price.price_per_troy_oz)
                    .map_err(|_| storage("provider returned an unparseable price"))?;
                let currency =
                    Currency::new(&price.currency).map_err(|e| storage(e.to_string()))?;

                spot::record_spot(
                    vault,
                    SpotReading {
                        metal,
                        price_per_troy_oz: value,
                        currency: &currency,
                        source: METALS_PROVIDER,
                        source_asof: &price.source_asof,
                        // One HTTP request fetched all four metals, so only
                        // the first charges the budget — but every price
                        // keeps API provenance, since that is its source.
                        origin: if updated.is_empty() {
                            SpotOrigin::Api
                        } else {
                            SpotOrigin::ApiSameRequest
                        },
                    },
                    &timestamp,
                )
                .map_err(storage)?;
                updated.push(metal.display_name().to_string());
            }

            Ok(RefreshResult {
                updated,
                source_asof: source_asof.clone(),
                quota: spot::quota_status(vault, METALS_PROVIDER, &timestamp)
                    .map_err(storage)?,
            })
        })
        .map_err(IpcError::from)
}
