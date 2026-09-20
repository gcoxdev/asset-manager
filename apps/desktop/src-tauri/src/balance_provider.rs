//! Watch-only balance lookup.
//!
//! # The disclosure, stated before anything else
//!
//! Querying an explorer tells it that whoever asked is interested in that
//! address, and links the address to an IP. For a public ledger the balance
//! was already public; the **association is the new information**, and it is
//! not recoverable once made.
//!
//! That is why this is gated behind its own opt-in rather than folded into
//! the price-feed setting. Someone who accepted price fetching has not
//! thereby accepted address disclosure — those are different facts about
//! them, revealed to different parties.
//!
//! # Never anything that can move funds
//!
//! Addresses are validated by `am_core::watch_only`, which refuses seed
//! phrases, extended private keys and WIF keys *before* checking format. This
//! module only ever sees something that already passed that check.
//!
//! # Verified terms (2026-09-20)
//!
//! Blockstream's Esplora API serves `GET /api/address/:addr` with no API key.
//! The response shape was checked against a live query:
//! `chain_stats.funded_txo_sum - chain_stats.spent_txo_sum` is the confirmed
//! balance in satoshis, and `mempool_stats` carries the unconfirmed delta.

use std::time::Duration;

use am_core::{Chain, WatchAddress};
use serde::{Deserialize, Serialize};

const ESPLORA_BASE: &str = "https://blockstream.info/api";
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, thiserror::Error)]
pub enum BalanceError {
    #[error("balance lookup is turned off — enable it in settings first")]
    NotEnabled,
    #[error("could not reach the block explorer: {0}")]
    Network(String),
    #[error("the block explorer rejected the request: {0}")]
    Rejected(String),
    #[error("the block explorer returned a response this build cannot read: {0}")]
    Malformed(String),
    #[error("balance lookup for {0} is not implemented yet")]
    UnsupportedChain(String),
}

#[derive(Debug, Deserialize)]
struct EsploraAddress {
    chain_stats: EsploraStats,
    #[serde(default)]
    mempool_stats: Option<EsploraStats>,
}

#[derive(Debug, Deserialize, Default)]
struct EsploraStats {
    #[serde(default)]
    funded_txo_sum: i64,
    #[serde(default)]
    spent_txo_sum: i64,
    #[serde(default)]
    tx_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddressBalance {
    pub address: String,
    pub chain: String,
    pub label: String,
    /// Confirmed balance as an exact decimal string in whole coins. Satoshis
    /// are converted by string manipulation rather than float division, which
    /// would lose precision at large balances.
    pub confirmed: String,
    /// Unconfirmed delta, which may be negative.
    pub unconfirmed: String,
    pub tx_count: i64,
    /// Ticker for display.
    pub unit: String,
    /// Which explorer answered, so a figure can be attributed.
    pub source: String,
}

/// Convert satoshis to a whole-coin decimal string.
///
/// Done on the digits rather than through `sats as f64 / 1e8`. For Bitcoin
/// specifically the float path would in fact be exact — f64 holds integers
/// to 2^53, about 90 million BTC in satoshis, comfortably above the 21
/// million supply — so this is not fixing a live bug.
///
/// It is still what should be written. The correctness of the float version
/// depends on a supply cap that is a property of one chain, not of this
/// function, and the first chain with smaller units or a larger supply would
/// break it silently. Integer arithmetic costs nothing and does not carry
/// that footnote.
fn sats_to_btc(sats: i64) -> String {
    let negative = sats < 0;
    let magnitude = sats.unsigned_abs().to_string();
    let padded = format!("{magnitude:0>9}");
    let split = padded.len() - 8;
    let whole = &padded[..split];
    let frac = padded[split..].trim_end_matches('0');

    let body = if frac.is_empty() { whole.to_string() } else { format!("{whole}.{frac}") };
    if negative {
        format!("-{body}")
    } else {
        body
    }
}

/// Look up a balance.
///
/// The caller is responsible for confirming the opt-in is on; this refuses
/// without it as a second guard rather than the only one.
pub fn fetch_balance(
    address: &WatchAddress,
    enabled: bool,
) -> Result<AddressBalance, BalanceError> {
    if !enabled {
        return Err(BalanceError::NotEnabled);
    }

    match address.chain {
        Chain::Bitcoin => fetch_bitcoin(address),
        // Ethereum balances need a different API and, for tokens, per-contract
        // calls. Refusing plainly beats a half-working implementation that
        // reports zero for a funded address.
        Chain::Ethereum => Err(BalanceError::UnsupportedChain("Ethereum".into())),
    }
}

fn fetch_bitcoin(address: &WatchAddress) -> Result<AddressBalance, BalanceError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent("asset-manager/0.0.1")
        .build()
        .map_err(|e| BalanceError::Network(e.to_string()))?;

    let url = format!("{ESPLORA_BASE}/address/{}", address.address);
    let response = client.get(&url).send().map_err(|e| BalanceError::Network(e.to_string()))?;

    let status = response.status();
    let body = response.text().map_err(|e| BalanceError::Network(e.to_string()))?;

    if status == reqwest::StatusCode::NOT_FOUND {
        // An address with no history is not an error; it holds nothing.
        return Ok(AddressBalance {
            address: address.address.clone(),
            chain: address.chain.as_str().to_string(),
            label: address.label.clone(),
            confirmed: "0".into(),
            unconfirmed: "0".into(),
            tx_count: 0,
            unit: "BTC".into(),
            source: "blockstream.info".into(),
        });
    }
    if !status.is_success() {
        return Err(BalanceError::Rejected(format!("HTTP {}", status.as_u16())));
    }

    parse_esplora(&body, address)
}

fn parse_esplora(body: &str, address: &WatchAddress) -> Result<AddressBalance, BalanceError> {
    let parsed: EsploraAddress =
        serde_json::from_str(body).map_err(|e| BalanceError::Malformed(e.to_string()))?;

    let confirmed = parsed.chain_stats.funded_txo_sum - parsed.chain_stats.spent_txo_sum;
    let mempool = parsed.mempool_stats.unwrap_or_default();
    let unconfirmed = mempool.funded_txo_sum - mempool.spent_txo_sum;

    Ok(AddressBalance {
        address: address.address.clone(),
        chain: address.chain.as_str().to_string(),
        label: address.label.clone(),
        confirmed: sats_to_btc(confirmed),
        unconfirmed: sats_to_btc(unconfirmed),
        tx_count: parsed.chain_stats.tx_count + mempool.tx_count,
        unit: "BTC".into(),
        source: "blockstream.info".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_core::parse_address;

    fn btc_address() -> WatchAddress {
        parse_address(Chain::Bitcoin, "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa", "test").unwrap()
    }

    #[test]
    fn satoshi_conversion_is_exact() {
        assert_eq!(sats_to_btc(100_000_000), "1");
        assert_eq!(sats_to_btc(150_000_000), "1.5");
        assert_eq!(sats_to_btc(1), "0.00000001");
        assert_eq!(sats_to_btc(0), "0");
        assert_eq!(sats_to_btc(-5_000_000), "-0.05");
    }

    #[test]
    fn large_balances_do_not_lose_precision() {
        // The whole Bitcoin supply plus one satoshi.
        assert_eq!(sats_to_btc(2_100_000_000_000_001), "21000000.00000001");

        // Above 2^53 satoshis the float path starts dropping the last digit.
        // Bitcoin never reaches this, but the integer path does not care:
        //   9007199254740993 sats -> f64 gives ...92, exact gives ...93
        assert_eq!(sats_to_btc(9_007_199_254_740_993), "90071992.54740993");
        let via_float = format!("{:.8}", 9_007_199_254_740_993_i64 as f64 / 1e8);
        assert_eq!(
            via_float, "90071992.54740992",
            "float loses the last satoshi past 2^53, which is why this uses integers"
        );
    }

    #[test]
    fn parses_the_verified_response_shape() {
        // Taken from a live query against blockstream.info for the genesis
        // address, so this pins the real field names rather than guessed ones.
        let body = r#"{
            "address": "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
            "chain_stats": {
                "funded_txo_count": 79517, "funded_txo_sum": 5747452472,
                "spent_txo_count": 0, "spent_txo_sum": 0, "tx_count": 66526
            },
            "mempool_stats": {
                "funded_txo_count": 4, "funded_txo_sum": 5310,
                "spent_txo_count": 0, "spent_txo_sum": 0, "tx_count": 4
            }
        }"#;

        let balance = parse_esplora(body, &btc_address()).unwrap();
        assert_eq!(balance.confirmed, "57.47452472");
        assert_eq!(balance.unconfirmed, "0.0000531");
        assert_eq!(balance.tx_count, 66_530);
        assert_eq!(balance.unit, "BTC");
    }

    #[test]
    fn a_spent_address_nets_to_its_remaining_balance() {
        let body = r#"{
            "chain_stats": {"funded_txo_sum": 500000000, "spent_txo_sum": 300000000, "tx_count": 10}
        }"#;
        let balance = parse_esplora(body, &btc_address()).unwrap();
        assert_eq!(balance.confirmed, "2", "5 BTC in, 3 out, 2 remaining");
        assert_eq!(balance.unconfirmed, "0", "absent mempool_stats is zero, not an error");
    }

    #[test]
    fn an_outgoing_unconfirmed_spend_is_negative() {
        let body = r#"{
            "chain_stats": {"funded_txo_sum": 100000000, "spent_txo_sum": 0, "tx_count": 1},
            "mempool_stats": {"funded_txo_sum": 0, "spent_txo_sum": 25000000, "tx_count": 1}
        }"#;
        let balance = parse_esplora(body, &btc_address()).unwrap();
        assert_eq!(balance.unconfirmed, "-0.25");
    }

    #[test]
    fn unknown_fields_do_not_break_parsing() {
        let body = r#"{
            "address": "x",
            "chain_stats": {"funded_txo_sum": 1, "spent_txo_sum": 0, "tx_count": 1, "new_field": 9},
            "something_added_upstream": true
        }"#;
        assert!(parse_esplora(body, &btc_address()).is_ok());
    }

    #[test]
    fn a_malformed_response_is_reported() {
        assert!(matches!(
            parse_esplora("not json", &btc_address()),
            Err(BalanceError::Malformed(_))
        ));
    }

    #[test]
    fn lookup_refuses_while_the_opt_in_is_off() {
        // A second guard, not the only one: the command checks too.
        assert!(matches!(fetch_balance(&btc_address(), false), Err(BalanceError::NotEnabled)));
    }

    #[test]
    fn ethereum_is_refused_plainly_rather_than_returning_zero() {
        // Reporting zero for a funded address would be worse than refusing.
        let eth = parse_address(
            Chain::Ethereum,
            "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            "test",
        )
        .unwrap();
        assert!(matches!(fetch_balance(&eth, true), Err(BalanceError::UnsupportedChain(_))));
    }
}
