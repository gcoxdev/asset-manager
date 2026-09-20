//! IPC for cryptocurrency holdings.
//!
//! Crypto gets a far more relaxed policy than metals because the budgets
//! differ by two orders of magnitude: CoinGecko's Demo plan allows 10,000
//! credits a month against metals.dev's 100. Reusing the metals quota logic
//! here would throttle for no reason.
//!
//! CoinGecko requires attribution wherever its data appears, so every
//! response carries the attribution string for the UI to display.

use am_core::{parse_decimal, CoinIdentity, Currency, Custody, COMMON_COINS};
use am_storage::valuations::{self, MatchQuality, NewQuote};
use am_storage::vault::VaultError;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::crypto_provider::{self, ATTRIBUTION};
use crate::session::{IpcError, Session, SessionError};

type IpcResult<T> = Result<T, IpcError>;

const PROVIDER: &str = "coingecko";

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
pub struct CoinOption {
    pub coin_id: String,
    pub symbol: String,
    pub name: String,
}

/// The bundled shortlist of well-known coins.
///
/// Short on purpose: it covers what people actually hold, and anything else
/// needs a real coin ID rather than a guessed symbol.
#[tauri::command]
pub fn common_coins() -> Vec<CoinOption> {
    COMMON_COINS
        .iter()
        .map(|(coin_id, symbol, name)| CoinOption {
            coin_id: (*coin_id).to_string(),
            symbol: (*symbol).to_string(),
            name: (*name).to_string(),
        })
        .collect()
}

#[derive(Serialize)]
pub struct CryptoProviderStatus {
    pub configured: bool,
    pub attribution: String,
}

#[tauri::command]
pub fn crypto_provider_status(session: State<'_, Session>) -> IpcResult<CryptoProviderStatus> {
    session.touch();
    Ok(CryptoProviderStatus {
        configured: crypto_provider::has_api_key(),
        attribution: ATTRIBUTION.to_string(),
    })
}

#[tauri::command]
pub fn set_crypto_api_key(session: State<'_, Session>, key: String) -> IpcResult<()> {
    session.touch();
    // Configuring a provider is an owner action; the lock establishes the
    // owner is present, even though the key itself lives in the keyring.
    session.with_vault(|_| Ok(())).map_err(IpcError::from)?;

    if key.trim().is_empty() {
        crypto_provider::clear_api_key()
            .map_err(|e| IpcError { kind: "keyring".into(), message: e.to_string() })
    } else {
        crypto_provider::store_api_key(&key)
            .map_err(|e| IpcError { kind: "keyring".into(), message: e.to_string() })
    }
}

#[derive(Deserialize)]
pub struct CryptoHoldingInput {
    pub coin_id: String,
    pub symbol: String,
    pub quantity: String,
    pub custody: Option<String>,
    pub location: Option<String>,
    pub chain: Option<String>,
    pub contract: Option<String>,
}

#[derive(Serialize)]
pub struct CryptoValuation {
    pub coin_id: String,
    pub label: String,
    pub quantity: String,
    pub unit_price: Option<String>,
    pub value: Option<String>,
    pub currency: String,
    pub source_asof: Option<String>,
    /// Present when no price could be found, so the UI can say why rather
    /// than showing a blank.
    pub unpriced_reason: Option<String>,
    pub attribution: String,
}

/// Value a crypto holding at the last stored price.
#[tauri::command]
pub fn value_crypto_holding(
    session: State<'_, Session>,
    holding: CryptoHoldingInput,
) -> IpcResult<CryptoValuation> {
    session.touch();

    let identity = match (holding.chain.as_deref(), holding.contract.as_deref()) {
        (Some(chain), Some(contract)) if !chain.is_empty() && !contract.is_empty() => {
            CoinIdentity::token(&holding.coin_id, &holding.symbol, chain, contract)
        }
        _ => CoinIdentity::native(&holding.coin_id, &holding.symbol),
    }
    .map_err(|e| bad_input(e.to_string()))?;

    let quantity =
        parse_decimal(&holding.quantity).map_err(|_| bad_input("quantity is not a number"))?;

    let core_holding = am_core::CryptoHolding {
        identity: identity.clone(),
        quantity,
        custody: holding
            .custody
            .as_deref()
            .and_then(Custody::parse)
            .unwrap_or(Custody::SelfCustody),
        location: holding.location.clone(),
    };

    session
        .with_vault(|vault| {
            let latest =
                valuations::latest_quote(vault, &identity.instrument_id()).map_err(storage)?;

            let Some((_quote_id, unit_price, currency, asof)) = latest else {
                return Ok(CryptoValuation {
                    coin_id: identity.coin_id.clone(),
                    label: identity.display_label(),
                    quantity: quantity.to_string(),
                    unit_price: None,
                    value: None,
                    currency: "USD".into(),
                    source_asof: None,
                    unpriced_reason: Some(
                        "no price stored for this coin — update prices or enter one by hand"
                            .into(),
                    ),
                    attribution: ATTRIBUTION.to_string(),
                });
            };

            let value = core_holding
                .value(unit_price, currency.clone())
                .map_err(|e| storage(e.to_string()))?;

            Ok(CryptoValuation {
                coin_id: identity.coin_id.clone(),
                label: identity.display_label(),
                quantity: quantity.to_string(),
                unit_price: Some(unit_price.to_string()),
                value: Some(value.format()),
                currency: currency.code().to_string(),
                source_asof: Some(asof),
                unpriced_reason: None,
                attribution: ATTRIBUTION.to_string(),
            })
        })
        .map_err(IpcError::from)
}

#[derive(Serialize)]
pub struct CryptoRefreshResult {
    pub updated: Vec<String>,
    /// Coins that were requested but came back unpriced, named so the user
    /// can tell a wrong ID from a provider outage.
    pub missing: Vec<String>,
    pub attribution: String,
}

/// Fetch prices for the given coin IDs in one request.
#[tauri::command]
pub fn refresh_crypto_prices(
    session: State<'_, Session>,
    coin_ids: Vec<String>,
) -> IpcResult<CryptoRefreshResult> {
    session.touch();
    let timestamp = now();

    // Normalize and de-duplicate before asking: sending the same id twice
    // wastes budget and returns nothing extra.
    let mut wanted: Vec<String> = coin_ids
        .iter()
        .map(|id| id.trim().to_ascii_lowercase())
        .filter(|id| !id.is_empty())
        .collect();
    wanted.sort();
    wanted.dedup();

    if wanted.is_empty() {
        return Ok(CryptoRefreshResult {
            updated: Vec::new(),
            missing: Vec::new(),
            attribution: ATTRIBUTION.to_string(),
        });
    }

    // Network call outside the vault lock: a slow response must not hold the
    // database open for its duration.
    let prices = crypto_provider::fetch_prices(&wanted, "usd").map_err(|e| IpcError {
        kind: match e {
            crypto_provider::CryptoProviderError::NoKey => "no_api_key",
            crypto_provider::CryptoProviderError::RateLimited => "rate_limited",
            _ => "provider_error",
        }
        .into(),
        message: e.to_string(),
    })?;

    let returned: Vec<String> = prices.iter().map(|p| p.coin_id.clone()).collect();
    let missing: Vec<String> =
        wanted.iter().filter(|id| !returned.contains(id)).cloned().collect();

    session
        .with_vault(|vault| {
            let mut updated = Vec::new();
            for price in &prices {
                let value = parse_decimal(&price.price)
                    .map_err(|_| storage("provider returned an unparseable price"))?;
                let currency =
                    Currency::new(&price.currency).map_err(|e| storage(e.to_string()))?;

                valuations::record_quote(
                    vault,
                    &NewQuote {
                        instrument_id: format!("coingecko:{}", price.coin_id),
                        unit_quote: value,
                        currency,
                        quote_unit: "coin".into(),
                        source: PROVIDER.into(),
                        match_quality: MatchQuality::Exact,
                        source_asof: price.source_asof.clone(),
                    },
                    &timestamp,
                )
                .map_err(storage)?;
                updated.push(price.coin_id.clone());
            }

            Ok(CryptoRefreshResult {
                updated,
                missing: missing.clone(),
                attribution: ATTRIBUTION.to_string(),
            })
        })
        .map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_coin_list_is_exposed() {
        let coins = common_coins();
        assert_eq!(coins.len(), COMMON_COINS.len());
        assert!(coins.iter().any(|c| c.coin_id == "bitcoin" && c.symbol == "BTC"));
    }

    #[test]
    fn every_exposed_coin_has_all_three_fields() {
        for coin in common_coins() {
            assert!(!coin.coin_id.is_empty());
            assert!(!coin.symbol.is_empty());
            assert!(!coin.name.is_empty(), "coin {} has no display name", coin.coin_id);
        }
    }
}
