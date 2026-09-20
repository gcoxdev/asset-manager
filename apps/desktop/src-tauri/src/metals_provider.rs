//! metals.dev price fetching.
//!
//! The only network access in the application. Everything else works offline,
//! and this is optional — the app is fully usable with hand-entered prices.
//!
//! # Verified terms (2026-09-20)
//!
//! Free tier: **100 requests per month**, 60-second updates, API key required,
//! no credit card. That budget is why `am-core::QuotaBudget` splits automatic
//! polling (25) from manual refresh (75), and why polling runs on a ~30-hour
//! interval rather than daily.
//!
//! # The API key
//!
//! Stored in the **OS keyring**, never in the vault and never in a dotfile.
//! Two reasons: a key in the vault would be unreadable while locked, when a
//! background refresh might want it; and a key in a config file is the classic
//! way secrets reach a backup or a screenshot.

use std::time::Duration;

use serde::{Deserialize, Serialize};

const SERVICE: &str = "dev.gcox.assetmanager";
const KEY_ENTRY: &str = "metals.dev-api-key";
const ENDPOINT: &str = "https://api.metals.dev/v1/latest";

/// Bounded so a hung connection cannot wedge a request that holds the vault
/// lock behind it.
const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("no metals.dev API key is configured")]
    NoKey,
    #[error("could not reach metals.dev: {0}")]
    Network(String),
    #[error("metals.dev rejected the request: {0}")]
    Rejected(String),
    #[error("metals.dev returned a response this build cannot read: {0}")]
    Malformed(String),
    #[error("the monthly request budget is spent; enter a price by hand instead")]
    QuotaExhausted,
    #[error("could not use the system keyring: {0}")]
    Keyring(String),
}

/// Store the API key in the OS keyring.
pub fn store_api_key(key: &str) -> Result<(), ProviderError> {
    let entry = keyring::Entry::new(SERVICE, KEY_ENTRY)
        .map_err(|e| ProviderError::Keyring(e.to_string()))?;
    entry.set_password(key.trim()).map_err(|e| ProviderError::Keyring(e.to_string()))
}

pub fn clear_api_key() -> Result<(), ProviderError> {
    let entry = keyring::Entry::new(SERVICE, KEY_ENTRY)
        .map_err(|e| ProviderError::Keyring(e.to_string()))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        // Already absent is the desired end state, not a failure.
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(ProviderError::Keyring(e.to_string())),
    }
}

pub fn has_api_key() -> bool {
    load_api_key().is_ok()
}

fn load_api_key() -> Result<String, ProviderError> {
    let entry = keyring::Entry::new(SERVICE, KEY_ENTRY)
        .map_err(|e| ProviderError::Keyring(e.to_string()))?;
    match entry.get_password() {
        Ok(key) if !key.trim().is_empty() => Ok(key),
        Ok(_) => Err(ProviderError::NoKey),
        Err(keyring::Error::NoEntry) => Err(ProviderError::NoKey),
        Err(e) => Err(ProviderError::Keyring(e.to_string())),
    }
}

/// The subset of the response this app reads.
///
/// Deliberately partial: metals.dev returns industrial metals, exchange-
/// specific rates and 170-odd currencies, none of which are wanted here.
/// Ignoring unknown fields also means an upstream addition cannot break
/// parsing.
#[derive(Debug, Deserialize)]
struct LatestResponse {
    status: String,
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    metals: Option<Metals>,
    #[serde(default)]
    timestamps: Option<Timestamps>,
    /// Present on failure.
    #[serde(default)]
    error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Metals {
    #[serde(default)]
    gold: Option<f64>,
    #[serde(default)]
    silver: Option<f64>,
    #[serde(default)]
    platinum: Option<f64>,
    #[serde(default)]
    palladium: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct Timestamps {
    #[serde(default)]
    metal: Option<String>,
}

/// One fetched price, as text.
///
/// Prices leave this module as **strings**, not floats. The JSON arrives as a
/// float because that is what the API sends, but it is converted once here and
/// never used for arithmetic — every calculation downstream is exact decimal.
#[derive(Debug, Clone, Serialize)]
pub struct FetchedPrice {
    /// ISO 4217 commodity code: XAU, XAG, XPT, XPD.
    pub metal: String,
    pub price_per_troy_oz: String,
    pub currency: String,
    /// The source's own timestamp, which is what staleness is judged against.
    pub source_asof: String,
}

/// Fetch all four precious metals in **one** request.
///
/// Batching is not an optimization here, it is the budget: four separate
/// requests would burn four of a hundred monthly calls for the same data.
pub fn fetch_spot_prices(currency: &str) -> Result<Vec<FetchedPrice>, ProviderError> {
    let api_key = load_api_key()?;

    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent("asset-manager/0.0.1")
        .build()
        .map_err(|e| ProviderError::Network(e.to_string()))?;

    let response = client
        .get(ENDPOINT)
        .query(&[("api_key", api_key.as_str()), ("currency", currency), ("unit", "toz")])
        .send()
        .map_err(|e| {
            // The key must never reach an error string that might be logged
            // or shown; reqwest includes the URL in some errors.
            ProviderError::Network(redact(&e.to_string(), &api_key))
        })?;

    let status = response.status();
    let body = response
        .text()
        .map_err(|e| ProviderError::Network(redact(&e.to_string(), &api_key)))?;

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(ProviderError::QuotaExhausted);
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(ProviderError::Rejected(
            "the API key was not accepted — check it in settings".into(),
        ));
    }
    if !status.is_success() {
        return Err(ProviderError::Rejected(format!("HTTP {}", status.as_u16())));
    }

    let parsed: LatestResponse = serde_json::from_str(&body)
        .map_err(|e| ProviderError::Malformed(redact(&e.to_string(), &api_key)))?;

    if parsed.status != "success" {
        return Err(ProviderError::Rejected(
            parsed.error_message.unwrap_or_else(|| parsed.status.clone()),
        ));
    }

    let metals = parsed
        .metals
        .ok_or_else(|| ProviderError::Malformed("response contained no metal prices".into()))?;
    let currency = parsed.currency.unwrap_or_else(|| currency.to_string());
    let asof = parsed
        .timestamps
        .and_then(|t| t.metal)
        // Without a source timestamp, staleness cannot be judged honestly, so
        // the response is refused rather than stamped with our own clock.
        .ok_or_else(|| {
            ProviderError::Malformed("response carried no metal timestamp".into())
        })?;

    let mut prices = Vec::new();
    for (code, value) in [
        ("XAU", metals.gold),
        ("XAG", metals.silver),
        ("XPT", metals.platinum),
        ("XPD", metals.palladium),
    ] {
        let Some(value) = value else { continue };
        if !value.is_finite() || value <= 0.0 {
            continue; // a nonsense price is skipped, not stored
        }
        prices.push(FetchedPrice {
            metal: code.to_string(),
            price_per_troy_oz: format_price(value),
            currency: currency.clone(),
            source_asof: asof.clone(),
        });
    }

    if prices.is_empty() {
        return Err(ProviderError::Malformed("no usable prices in the response".into()));
    }
    Ok(prices)
}

/// Render an API float as a decimal string without scientific notation.
///
/// Six decimal places is beyond any metal's quoted precision and avoids
/// `1e3`-style output, which would fail to parse downstream.
fn format_price(value: f64) -> String {
    let formatted = format!("{value:.6}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Remove the API key from any text that might be surfaced or logged.
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
    fn prices_format_without_scientific_notation() {
        assert_eq!(format_price(2014.3), "2014.3");
        assert_eq!(format_price(30.0), "30");
        assert_eq!(format_price(0.000123), "0.000123");
        // A large value must not come back as 1e6.
        assert_eq!(format_price(1_000_000.0), "1000000");
        assert!(!format_price(1e-7).contains('e'));
    }

    #[test]
    fn formatted_prices_parse_back_as_exact_decimals() {
        // The handoff that matters: the float stops here and everything
        // downstream is exact.
        for value in [2014.3_f64, 30.125, 0.5, 999.999] {
            let text = format_price(value);
            assert!(
                am_core::parse_decimal(&text).is_ok(),
                "{text} is not parseable as a decimal"
            );
        }
    }

    #[test]
    fn redaction_removes_the_key() {
        let message = "error sending request for url (https://api.metals.dev/v1/latest?api_key=SECRET123)";
        let cleaned = redact(message, "SECRET123");
        assert!(!cleaned.contains("SECRET123"));
        assert!(cleaned.contains("[redacted]"));
    }

    #[test]
    fn redaction_is_safe_with_an_empty_key() {
        assert_eq!(redact("some error", ""), "some error");
    }

    #[test]
    fn a_failure_response_is_reported_not_parsed_as_prices() {
        let body = r#"{"status":"failure","error_message":"Invalid API key"}"#;
        let parsed: LatestResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.status, "failure");
        assert_eq!(parsed.error_message.as_deref(), Some("Invalid API key"));
        assert!(parsed.metals.is_none());
    }

    #[test]
    fn unknown_fields_do_not_break_parsing() {
        // metals.dev returns industrial metals and 170 currencies; an
        // upstream addition must not break the client.
        let body = r#"{
            "status":"success",
            "currency":"USD",
            "unit":"toz",
            "metals":{"gold":2014.3,"silver":30.1,"copper":4.2,"some_new_metal":1.0},
            "currencies":{"EUR":0.92,"GBP":0.79},
            "timestamps":{"metal":"2026-09-20T06:16:02.829Z","currency":"2026-09-20T06:16:02.829Z"},
            "an_entirely_new_field":{"nested":true}
        }"#;
        let parsed: LatestResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.status, "success");
        let metals = parsed.metals.unwrap();
        assert_eq!(metals.gold, Some(2014.3));
        assert_eq!(metals.platinum, None, "absent metals are None, not an error");
    }

    #[test]
    fn a_response_without_metals_is_malformed() {
        let body = r#"{"status":"success","currency":"USD"}"#;
        let parsed: LatestResponse = serde_json::from_str(body).unwrap();
        assert!(parsed.metals.is_none());
        assert!(parsed.timestamps.is_none());
    }

    #[test]
    fn missing_and_nonsense_prices_are_skipped() {
        // Mirrors the filter in fetch_spot_prices: a zero or NaN price is
        // worse than no price, because it would silently zero a holding.
        for bad in [0.0_f64, -1.0, f64::NAN, f64::INFINITY] {
            assert!(!(bad.is_finite() && bad > 0.0), "{bad} should have been rejected");
        }
        assert!(2014.3_f64.is_finite() && 2014.3 > 0.0);
    }
}
