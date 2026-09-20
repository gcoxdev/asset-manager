//! CoinGecko price fetching.
//!
//! # Verified terms (2026-09-20)
//!
//! Demo (free) plan: base URL `https://api.coingecko.com/api/v3`, key sent as
//! the `x-cg-demo-api-key` header, **10,000 call credits per month** and
//! **100 requests per minute**, no credit card. **Attribution is required**
//! by CoinGecko's branding guidelines — see [`ATTRIBUTION`], which the UI
//! displays whenever CoinGecko data is shown.
//!
//! That budget is two orders of magnitude larger than metals.dev's 100/month,
//! so crypto gets a much more relaxed polling policy than metals. The shared
//! `QuotaBudget` type is deliberately *not* reused here — applying a
//! 100-request budget to a 10,000-credit allowance would throttle for no
//! reason.
//!
//! # One request, many coins
//!
//! `/simple/price` accepts comma-separated IDs, so an entire portfolio prices
//! in a single call. Batching is the design, not an optimization: per-coin
//! requests would multiply credit use by the number of holdings.

use std::collections::HashMap;
use std::time::Duration;

use serde::Serialize;

const SERVICE: &str = "dev.gcox.assetmanager";
const KEY_ENTRY: &str = "coingecko-api-key";
const ENDPOINT: &str = "https://api.coingecko.com/api/v3/simple/price";
const TIMEOUT: Duration = Duration::from_secs(15);

/// Required by CoinGecko's branding guidelines wherever their data appears.
pub const ATTRIBUTION: &str = "Price data by CoinGecko";

/// CoinGecko's documented cap for a single `/simple/price` call.
const MAX_IDS_PER_REQUEST: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum CryptoProviderError {
    #[error("no CoinGecko API key is configured")]
    NoKey,
    #[error("could not reach CoinGecko: {0}")]
    Network(String),
    #[error("CoinGecko rejected the request: {0}")]
    Rejected(String),
    #[error("CoinGecko returned a response this build cannot read: {0}")]
    Malformed(String),
    #[error("CoinGecko rate limit reached — wait a minute and try again")]
    RateLimited,
    #[error("could not use the system keyring: {0}")]
    Keyring(String),
}

pub fn store_api_key(key: &str) -> Result<(), CryptoProviderError> {
    let entry = keyring::Entry::new(SERVICE, KEY_ENTRY)
        .map_err(|e| CryptoProviderError::Keyring(e.to_string()))?;
    entry
        .set_password(key.trim())
        .map_err(|e| CryptoProviderError::Keyring(e.to_string()))
}

pub fn clear_api_key() -> Result<(), CryptoProviderError> {
    let entry = keyring::Entry::new(SERVICE, KEY_ENTRY)
        .map_err(|e| CryptoProviderError::Keyring(e.to_string()))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(CryptoProviderError::Keyring(e.to_string())),
    }
}

pub fn has_api_key() -> bool {
    load_api_key().is_ok()
}

fn load_api_key() -> Result<String, CryptoProviderError> {
    let entry = keyring::Entry::new(SERVICE, KEY_ENTRY)
        .map_err(|e| CryptoProviderError::Keyring(e.to_string()))?;
    match entry.get_password() {
        Ok(key) if !key.trim().is_empty() => Ok(key),
        Ok(_) => Err(CryptoProviderError::NoKey),
        Err(keyring::Error::NoEntry) => Err(CryptoProviderError::NoKey),
        Err(e) => Err(CryptoProviderError::Keyring(e.to_string())),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FetchedCryptoPrice {
    /// The coin ID that was requested, so the caller can match it back.
    pub coin_id: String,
    /// Decimal string — the float from the JSON stops here.
    pub price: String,
    pub currency: String,
    /// CoinGecko's own `last_updated_at`, as RFC 3339. Staleness is judged
    /// against this, never our fetch time.
    pub source_asof: String,
}

/// Fetch prices for several coins in one request.
///
/// Requesting more than [`MAX_IDS_PER_REQUEST`] would be silently truncated by
/// the API, leaving holdings unpriced with no error, so the list is chunked
/// and each chunk requested separately.
pub fn fetch_prices(
    coin_ids: &[String],
    currency: &str,
) -> Result<Vec<FetchedCryptoPrice>, CryptoProviderError> {
    if coin_ids.is_empty() {
        return Ok(Vec::new());
    }
    let api_key = load_api_key()?;
    let currency = currency.to_ascii_lowercase();

    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent("asset-manager/0.0.1")
        .build()
        .map_err(|e| CryptoProviderError::Network(e.to_string()))?;

    let mut out = Vec::new();
    for chunk in coin_ids.chunks(MAX_IDS_PER_REQUEST) {
        out.extend(fetch_chunk(&client, &api_key, chunk, &currency)?);
    }
    Ok(out)
}

fn fetch_chunk(
    client: &reqwest::blocking::Client,
    api_key: &str,
    coin_ids: &[String],
    currency: &str,
) -> Result<Vec<FetchedCryptoPrice>, CryptoProviderError> {
    let ids = coin_ids.join(",");

    let response = client
        .get(ENDPOINT)
        .header("x-cg-demo-api-key", api_key)
        .query(&[
            ("ids", ids.as_str()),
            ("vs_currencies", currency),
            // Needed to judge staleness honestly.
            ("include_last_updated_at", "true"),
            // Precision "full" avoids CoinGecko rounding a sub-cent token
            // price to zero before it ever reaches us.
            ("precision", "full"),
        ])
        .send()
        .map_err(|e| CryptoProviderError::Network(redact(&e.to_string(), api_key)))?;

    let status = response.status();
    let body = response
        .text()
        .map_err(|e| CryptoProviderError::Network(redact(&e.to_string(), api_key)))?;

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(CryptoProviderError::RateLimited);
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(CryptoProviderError::Rejected(
            "the API key was not accepted — check it in settings".into(),
        ));
    }
    if !status.is_success() {
        return Err(CryptoProviderError::Rejected(format!("HTTP {}", status.as_u16())));
    }

    parse_response(&body, currency).map_err(|e| match e {
        CryptoProviderError::Malformed(m) => {
            CryptoProviderError::Malformed(redact(&m, api_key))
        }
        other => other,
    })
}

/// Parse `{"bitcoin":{"usd":76975,"last_updated_at":1758...}}`.
///
/// Split out from the request so the shape can be tested without a network.
fn parse_response(
    body: &str,
    currency: &str,
) -> Result<Vec<FetchedCryptoPrice>, CryptoProviderError> {
    let parsed: HashMap<String, HashMap<String, serde_json::Value>> =
        serde_json::from_str(body)
            .map_err(|e| CryptoProviderError::Malformed(e.to_string()))?;

    let mut out = Vec::new();
    for (coin_id, fields) in parsed {
        let Some(price) = fields.get(currency).and_then(|v| v.as_f64()) else {
            // A coin CoinGecko does not know is simply absent from the
            // response. Skipping leaves the holding unpriced and visible as
            // such, which is better than inventing a number.
            continue;
        };
        if !price.is_finite() || price <= 0.0 {
            continue;
        }

        let updated = fields
            .get("last_updated_at")
            .and_then(|v| v.as_i64())
            .map(unix_to_rfc3339)
            // Without a source timestamp staleness cannot be judged, so the
            // price is dropped rather than stamped with our own clock.
            .ok_or_else(|| {
                CryptoProviderError::Malformed(format!(
                    "{coin_id} price carried no last_updated_at"
                ))
            })?;

        out.push(FetchedCryptoPrice {
            coin_id,
            price: format_price(price),
            currency: currency.to_ascii_uppercase(),
            source_asof: updated,
        });
    }

    out.sort_by(|a, b| a.coin_id.cmp(&b.coin_id));
    Ok(out)
}

/// Unix seconds to an RFC-3339 UTC string.
fn unix_to_rfc3339(seconds: i64) -> String {
    // days-from-civil, inverted. Exact, and avoids a date dependency here.
    let days = seconds.div_euclid(86_400);
    let secs_of_day = seconds.rem_euclid(86_400);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// Render an API float as a decimal string.
///
/// Twelve places rather than metals' six: a memecoin can trade at
/// 0.000000001234, and truncating that to six would zero the holding.
fn format_price(value: f64) -> String {
    let formatted = format!("{value:.12}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() { "0".to_string() } else { trimmed.to_string() }
}

fn redact(text: &str, api_key: &str) -> String {
    if api_key.is_empty() {
        return text.to_string();
    }
    text.replace(api_key, "[redacted]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_normal_response() {
        let body = r#"{
            "bitcoin":{"usd":76975.0,"last_updated_at":1758000000},
            "ethereum":{"usd":3120.5,"last_updated_at":1758000060}
        }"#;
        let prices = parse_response(body, "usd").unwrap();

        assert_eq!(prices.len(), 2);
        assert_eq!(prices[0].coin_id, "bitcoin", "results are sorted for stable output");
        assert_eq!(prices[0].price, "76975");
        assert_eq!(prices[0].currency, "USD");
        assert!(prices[0].source_asof.ends_with('Z'));
    }

    #[test]
    fn a_tiny_price_survives_with_full_precision() {
        // The case that makes 12 decimal places necessary.
        let body = r#"{"tiny-coin":{"usd":0.000000001234,"last_updated_at":1758000000}}"#;
        let prices = parse_response(body, "usd").unwrap();
        assert_eq!(prices[0].price, "0.000000001234");
        assert!(am_core::parse_decimal(&prices[0].price).is_ok());
    }

    #[test]
    fn an_unknown_coin_is_absent_rather_than_zero() {
        // CoinGecko omits ids it does not recognize. Skipping leaves the
        // holding visibly unpriced instead of inventing a value.
        let body = r#"{"bitcoin":{"usd":76975.0,"last_updated_at":1758000000}}"#;
        let prices = parse_response(body, "usd").unwrap();
        assert_eq!(prices.len(), 1);
        assert!(!prices.iter().any(|p| p.coin_id == "not-a-real-coin"));
    }

    #[test]
    fn a_price_without_a_timestamp_is_refused() {
        // Staleness judged against our own clock would be a lie.
        let body = r#"{"bitcoin":{"usd":76975.0}}"#;
        assert!(matches!(
            parse_response(body, "usd"),
            Err(CryptoProviderError::Malformed(_))
        ));
    }

    #[test]
    fn zero_and_negative_prices_are_skipped() {
        let body = r#"{
            "good":{"usd":1.5,"last_updated_at":1758000000},
            "zero":{"usd":0,"last_updated_at":1758000000},
            "negative":{"usd":-5,"last_updated_at":1758000000}
        }"#;
        let prices = parse_response(body, "usd").unwrap();
        assert_eq!(prices.len(), 1);
        assert_eq!(prices[0].coin_id, "good");
    }

    #[test]
    fn a_missing_currency_field_is_skipped_not_guessed() {
        // Asking for USD and getting only EUR must not be read as a USD price.
        let body = r#"{"bitcoin":{"eur":70000.0,"last_updated_at":1758000000}}"#;
        let prices = parse_response(body, "usd").unwrap();
        assert!(prices.is_empty());
    }

    #[test]
    fn unix_timestamps_convert_correctly() {
        assert_eq!(unix_to_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(unix_to_rfc3339(1_000_000_000), "2001-09-09T01:46:40Z");
        // A leap day.
        assert_eq!(unix_to_rfc3339(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn prices_never_use_scientific_notation() {
        for value in [1e-9_f64, 1e9, 76975.0, 0.000000001234] {
            let text = format_price(value);
            assert!(!text.contains('e'), "{text} used scientific notation");
            assert!(
                am_core::parse_decimal(&text).is_ok(),
                "{text} is not parseable as a decimal"
            );
        }
    }

    #[test]
    fn redaction_removes_the_key() {
        let cleaned = redact("failed: x-cg-demo-api-key=SECRET", "SECRET");
        assert!(!cleaned.contains("SECRET"));
    }

    #[test]
    fn attribution_text_is_present() {
        // CoinGecko's branding guidelines require it wherever their data is
        // shown; a build without it would be out of compliance.
        assert!(ATTRIBUTION.contains("CoinGecko"));
    }

    #[test]
    fn an_empty_request_makes_no_network_call() {
        // Verified by the early return: no key is needed, so this succeeds
        // even with none configured.
        assert!(fetch_prices(&[], "usd").unwrap().is_empty());
    }
}
