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
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::crypto_provider::{self, ATTRIBUTION};
use crate::ipc::{bad_input, base_currency, now, storage, IpcResult};
use crate::session::{IpcError, Session, SessionError};

const PROVIDER: &str = "coingecko";

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
    pub revalued: am_storage::pricing::RevalueSummary,
}

/// Fetch prices in one batched request, then revalue the holdings.
///
/// With no IDs given, asks only for coins actually held: every ID costs
/// provider credits, and asking about coins nobody owns would spend them —
/// and tell the provider more than it needs to know.
#[tauri::command]
pub fn refresh_crypto_prices(
    session: State<'_, Session>,
    coin_ids: Option<Vec<String>>,
) -> IpcResult<CryptoRefreshResult> {
    session.touch();
    let timestamp = now();

    let (held, currency) = session
        .with_vault(|vault| {
            Ok((
                am_storage::pricing::held_coin_ids(vault).map_err(storage)?,
                base_currency(vault),
            ))
        })
        .map_err(IpcError::from)?;

    // Normalize and de-duplicate before asking: sending the same id twice
    // wastes budget and returns nothing extra.
    let mut wanted: Vec<String> = coin_ids
        .unwrap_or(held)
        .iter()
        .map(|id| id.trim().to_ascii_lowercase())
        .filter(|id| !id.is_empty())
        .collect();
    wanted.sort();
    wanted.dedup();

    if wanted.is_empty() {
        return Err(bad_input("no crypto holdings to price yet — add one first"));
    }

    // Network call outside the vault lock: a slow response must not hold the
    // database open for its duration.
    let prices = crypto_provider::fetch_prices(&wanted, &currency.code().to_ascii_lowercase())
        .map_err(|e| IpcError {
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

            let revalued =
                am_storage::pricing::revalue_all(vault, &timestamp).map_err(storage)?;
            Ok(CryptoRefreshResult {
                updated,
                missing: missing.clone(),
                attribution: ATTRIBUTION.to_string(),
                revalued,
            })
        })
        .map_err(IpcError::from)
}

/// Enter a coin price by hand — for a coin the provider does not list, or
/// when no key is configured.
#[tauri::command]
pub fn set_coin_price(
    session: State<'_, Session>,
    coin_id: String,
    price: String,
) -> IpcResult<am_storage::pricing::RevalueSummary> {
    session.touch();
    let timestamp = now();
    let identity =
        CoinIdentity::native(&coin_id, &coin_id).map_err(|e| bad_input(e.to_string()))?;
    let cleaned: String = price.chars().filter(|c| *c != ',' && *c != '$').collect();
    let value = parse_decimal(&cleaned)
        .map_err(|_| bad_input(format!("{price:?} is not a valid price")))?;
    if value <= am_core::Decimal::ZERO {
        return Err(bad_input("a price must be greater than zero"));
    }

    session
        .with_vault(|vault| {
            valuations::record_quote(
                vault,
                &NewQuote {
                    instrument_id: identity.instrument_id(),
                    unit_quote: value,
                    currency: base_currency(vault),
                    quote_unit: "coin".into(),
                    source: "manual".into(),
                    match_quality: MatchQuality::Manual,
                    source_asof: timestamp.clone(),
                },
                &timestamp,
            )
            .map_err(storage)?;
            am_storage::pricing::revalue_all(vault, &timestamp).map_err(storage)
        })
        .map_err(IpcError::from)
}

#[derive(Serialize)]
pub struct CoinPrice {
    pub coin_id: String,
    pub symbol: String,
    pub name: String,
    /// Total held across every holding of this coin, exact decimal.
    pub held: String,
    pub unit_price: Option<String>,
    pub currency: Option<String>,
    pub source: Option<String>,
    pub source_asof: Option<String>,
}

/// Every held coin with its latest stored price.
#[tauri::command]
pub fn crypto_prices(session: State<'_, Session>) -> IpcResult<Vec<CoinPrice>> {
    session.touch();
    session
        .with_vault(|vault| {
            let mut stmt = vault
                .conn()
                .prepare(
                    "SELECT json_extract(attrs, '$.coin_id'), json_extract(attrs, '$.symbol'),
                            quantity
                     FROM assets
                     WHERE status = 'active' AND json_extract(attrs, '$.coin_id') IS NOT NULL",
                )
                .map_err(storage)?;
            let rows: Vec<(String, Option<String>, String)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(storage)?
                .collect::<Result<_, _>>()
                .map_err(storage)?;
            drop(stmt);

            let mut coins: Vec<CoinPrice> = Vec::new();
            for (coin_id, symbol, quantity) in rows {
                let qty = parse_decimal(&quantity).unwrap_or_default();
                if let Some(existing) = coins.iter_mut().find(|c| c.coin_id == coin_id) {
                    let total = parse_decimal(&existing.held).unwrap_or_default() + qty;
                    existing.held = total.normalize().to_string();
                    continue;
                }
                let known = am_core::crypto_assets::common_coin(&coin_id);
                let quote: Option<(String, String, String, String)> = vault
                    .conn()
                    .query_row(
                        "SELECT unit_quote, currency, source, source_asof FROM quotes
                         WHERE instrument_id = ?1
                         ORDER BY source_asof DESC, fetched_at DESC, rowid DESC LIMIT 1",
                        [format!("coingecko:{coin_id}")],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .ok();
                coins.push(CoinPrice {
                    symbol: symbol
                        .or_else(|| known.map(|k| k.1.to_string()))
                        .unwrap_or_else(|| coin_id.to_ascii_uppercase()),
                    name: known.map(|k| k.2.to_string()).unwrap_or_else(|| coin_id.clone()),
                    held: qty.normalize().to_string(),
                    unit_price: quote.as_ref().map(|q| q.0.clone()),
                    currency: quote.as_ref().map(|q| q.1.clone()),
                    source: quote.as_ref().map(|q| q.2.clone()),
                    source_asof: quote.map(|q| q.3),
                    coin_id,
                });
            }
            Ok(coins)
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

// ------------------------------------------------- watch-only balances

use am_core::{parse_address, Chain};

/// Where the balance-lookup opt-in is recorded.
///
/// In the vault rather than a config file, so it travels with the data it
/// governs and cannot be flipped on by editing a dotfile.
pub const BALANCE_OPT_IN_KEY: &str = "watch_only_balance_lookup";

fn balance_lookup_enabled(vault: &am_storage::vault::Vault) -> Result<bool, SessionError> {
    let value: Option<String> = vault
        .conn()
        .query_row("SELECT value FROM app_settings WHERE key = ?1", [BALANCE_OPT_IN_KEY], |r| {
            r.get(0)
        })
        .ok();
    Ok(value.as_deref() == Some("true"))
}

#[derive(Serialize)]
pub struct BalanceLookupStatus {
    pub enabled: bool,
    /// Shown next to the toggle. The disclosure is the point of the opt-in,
    /// so it is not hidden behind a help link.
    pub disclosure: String,
}

#[tauri::command]
pub fn balance_lookup_status(session: State<'_, Session>) -> IpcResult<BalanceLookupStatus> {
    session.touch();
    session
        .with_vault(|vault| {
            Ok(BalanceLookupStatus {
                enabled: balance_lookup_enabled(vault)?,
                disclosure: "Looking up a balance sends the address to a public block \
                             explorer, which learns that someone at your IP is \
                             interested in it. The balance itself is already public; \
                             the link to you is not. This is separate from price \
                             fetching and off by default."
                    .to_string(),
            })
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn set_balance_lookup(session: State<'_, Session>, enabled: bool) -> IpcResult<()> {
    session.touch();
    session
        .with_vault(|vault| {
            vault
                .conn()
                .execute(
                    "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    rusqlite::params![
                        BALANCE_OPT_IN_KEY,
                        if enabled { "true" } else { "false" }
                    ],
                )
                .map_err(storage)?;
            Ok(())
        })
        .map_err(IpcError::from)
}

/// Look up a watch-only balance.
///
/// Validates the address first, which refuses anything resembling a key or
/// seed phrase before it could reach the network.
#[tauri::command]
pub fn lookup_balance(
    session: State<'_, Session>,
    chain: String,
    address: String,
    label: Option<String>,
) -> IpcResult<crate::balance_provider::AddressBalance> {
    session.touch();

    let chain =
        Chain::parse(&chain).ok_or_else(|| bad_input(format!("unknown chain: {chain}")))?;
    let parsed = parse_address(chain, &address, label.as_deref().unwrap_or(""))
        .map_err(|e| bad_input(e.to_string()))?;

    let enabled = session.with_vault(balance_lookup_enabled).map_err(IpcError::from)?;

    // Checked here as well as in the provider: the opt-in is the whole
    // control, so it gets two guards rather than one.
    if !enabled {
        return Err(IpcError {
            kind: "not_enabled".into(),
            message: "balance lookup is off — turn it on in settings, noting that it \
                      discloses the address to a public explorer"
                .into(),
        });
    }

    crate::balance_provider::fetch_balance(&parsed, enabled).map_err(|e| IpcError {
        kind: match e {
            crate::balance_provider::BalanceError::NotEnabled => "not_enabled",
            crate::balance_provider::BalanceError::UnsupportedChain(_) => "unsupported_chain",
            _ => "lookup_failed",
        }
        .into(),
        message: e.to_string(),
    })
}
