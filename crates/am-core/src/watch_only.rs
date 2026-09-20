//! Watch-only address tracking.
//!
//! # What this is not
//!
//! **Never keys, never seed phrases, never anything that can move funds.**
//! An address is a public identifier. If a field would accept a private key
//! or mnemonic, it does not belong in this module — and a validator that
//! *rejects* things resembling secrets is part of the design, because the
//! common accident is pasting the wrong string into the right box.
//!
//! # The privacy cost, stated plainly
//!
//! Querying a balance tells the remote explorer that whoever asked is
//! interested in that address, and links it to an IP. For a chain with a
//! public ledger the balance was already public; the new information is the
//! association. That is why the plan gates this behind a separate opt-in
//! rather than folding it into the price-feed setting — a user who accepted
//! price fetching has not thereby accepted address disclosure.

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum WatchOnlyError {
    #[error("an address is required")]
    Empty,
    #[error(
        "that looks like a private key or recovery phrase, not an address — \
         this app never accepts anything that can move funds"
    )]
    LooksLikeSecret,
    #[error("{0:?} is not a recognized address format")]
    Unrecognized(String),
    #[error("address is too long")]
    TooLong,
}

/// Chains whose address formats this build recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Chain {
    Bitcoin,
    Ethereum,
}

impl Chain {
    pub fn as_str(self) -> &'static str {
        match self {
            Chain::Bitcoin => "bitcoin",
            Chain::Ethereum => "ethereum",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "bitcoin" | "btc" => Chain::Bitcoin,
            "ethereum" | "eth" => Chain::Ethereum,
            _ => return None,
        })
    }
}

/// A validated watch-only address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchAddress {
    pub chain: Chain,
    pub address: String,
    /// The owner's own name for it, e.g. "cold wallet".
    pub label: String,
}

const MAX_ADDRESS_LEN: usize = 120;

/// Heuristics for input that is a secret rather than an address.
///
/// Deliberately conservative — a false positive costs a confused user one
/// retry, while a false negative means a seed phrase in a database.
fn looks_like_secret(input: &str) -> bool {
    let lower = input.trim().to_ascii_lowercase();

    // A BIP-39 mnemonic is 12–24 space-separated words.
    let words: Vec<&str> = lower.split_whitespace().collect();
    if words.len() >= 12 && words.iter().all(|w| w.chars().all(|c| c.is_ascii_alphabetic())) {
        return true;
    }

    // An extended private key.
    if lower.starts_with("xprv") || lower.starts_with("yprv") || lower.starts_with("zprv") {
        return true;
    }

    // A bare 64-hex-character string is an Ethereum private key; addresses
    // are 40 hex characters after the 0x prefix.
    let hex_body = lower.strip_prefix("0x").unwrap_or(&lower);
    if hex_body.len() == 64 && hex_body.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }

    // WIF-encoded Bitcoin private keys.
    if (input.starts_with('5') && input.len() == 51)
        || ((input.starts_with('K') || input.starts_with('L')) && input.len() == 52)
    {
        return true;
    }

    false
}

/// Validate an address for a chain.
///
/// Format checking only. Whether the address exists or holds anything is a
/// question for an explorer, not for this.
pub fn parse_address(
    chain: Chain,
    raw: &str,
    label: &str,
) -> Result<WatchAddress, WatchOnlyError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(WatchOnlyError::Empty);
    }
    if trimmed.len() > MAX_ADDRESS_LEN {
        return Err(WatchOnlyError::TooLong);
    }
    // Checked before format, so a pasted secret is named as such rather than
    // reported as a malformed address.
    if looks_like_secret(trimmed) {
        return Err(WatchOnlyError::LooksLikeSecret);
    }

    let valid = match chain {
        Chain::Bitcoin => is_bitcoin_address(trimmed),
        Chain::Ethereum => is_ethereum_address(trimmed),
    };
    if !valid {
        return Err(WatchOnlyError::Unrecognized(trimmed.to_string()));
    }

    Ok(WatchAddress {
        chain,
        // Ethereum addresses are case-insensitive but carry an optional
        // checksum in their casing, so the original is preserved.
        address: trimmed.to_string(),
        label: label.trim().to_string(),
    })
}

/// Shape check for Bitcoin: legacy, P2SH, or bech32.
fn is_bitcoin_address(address: &str) -> bool {
    // bech32 / bech32m
    if let Some(rest) = address.strip_prefix("bc1") {
        return (14..=71).contains(&rest.len())
            && rest.chars().all(|c| c.is_ascii_alphanumeric() && !"1bio".contains(c));
    }
    // Base58Check: legacy (1) and P2SH (3). Base58 excludes 0, O, I and l.
    if address.starts_with('1') || address.starts_with('3') {
        return (26..=35).contains(&address.len())
            && address.chars().all(|c| c.is_ascii_alphanumeric() && !"0OIl".contains(c));
    }
    false
}

/// Shape check for Ethereum: `0x` and 40 hex characters.
fn is_ethereum_address(address: &str) -> bool {
    let Some(body) = address.strip_prefix("0x").or_else(|| address.strip_prefix("0X")) else {
        return false;
    };
    body.len() == 40 && body.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_real_bitcoin_address_formats() {
        for address in [
            "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa", // genesis, legacy
            "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy", // P2SH
            "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", // bech32
        ] {
            assert!(
                parse_address(Chain::Bitcoin, address, "test").is_ok(),
                "rejected valid address {address}"
            );
        }
    }

    #[test]
    fn accepts_ethereum_addresses_in_either_case() {
        let lower = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48";
        let mixed = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";

        assert!(parse_address(Chain::Ethereum, lower, "x").is_ok());
        let parsed = parse_address(Chain::Ethereum, mixed, "x").unwrap();
        assert_eq!(
            parsed.address, mixed,
            "checksum casing must be preserved, not normalized away"
        );
    }

    #[test]
    fn rejects_a_seed_phrase() {
        // The accident this exists to catch.
        let mnemonic = "abandon abandon abandon abandon abandon abandon \
                        abandon abandon abandon abandon abandon about";
        assert_eq!(
            parse_address(Chain::Bitcoin, mnemonic, "oops"),
            Err(WatchOnlyError::LooksLikeSecret)
        );
    }

    #[test]
    fn rejects_private_keys_of_several_shapes() {
        for secret in [
            // Extended private key.
            "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi",
            // 64 hex characters is an Ethereum private key, not an address.
            "0x4c0883a69102937d6231471b5dbb6204fe512961708279f1f2b1e8b2b0e0e0e0",
            // WIF.
            "5HueCGU8rMjxEXxiPuD5BDku4MkFqeZyd4dZ1jvhTVqvbTLvyTJ",
        ] {
            let result = parse_address(Chain::Ethereum, secret, "oops");
            assert_eq!(
                result,
                Err(WatchOnlyError::LooksLikeSecret),
                "accepted something secret-shaped: {secret}"
            );
        }
    }

    #[test]
    fn a_secret_is_named_as_such_not_reported_as_malformed() {
        // The message has to say what went wrong, or someone will "fix" it by
        // trying harder to paste the key.
        let err = parse_address(
            Chain::Bitcoin,
            "5HueCGU8rMjxEXxiPuD5BDku4MkFqeZyd4dZ1jvhTVqvbTLvyTJ",
            "x",
        )
        .unwrap_err();
        assert!(err.to_string().contains("never accepts"));
    }

    #[test]
    fn rejects_malformed_addresses() {
        assert!(parse_address(Chain::Bitcoin, "not-an-address", "x").is_err());
        assert!(parse_address(Chain::Ethereum, "0x123", "x").is_err(), "too short");
        assert!(
            parse_address(Chain::Ethereum, "a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48", "x")
                .is_err(),
            "missing 0x prefix"
        );
        // Base58 has no zero or capital O.
        assert!(parse_address(Chain::Bitcoin, "10OIl1eP5QGefi2DMPTfTL5SLmv7Div", "x").is_err());
    }

    #[test]
    fn rejects_empty_and_oversized_input() {
        assert_eq!(parse_address(Chain::Bitcoin, "   ", "x"), Err(WatchOnlyError::Empty));
        assert_eq!(
            parse_address(Chain::Bitcoin, &"1".repeat(MAX_ADDRESS_LEN + 1), "x"),
            Err(WatchOnlyError::TooLong)
        );
    }

    #[test]
    fn a_bitcoin_address_is_not_valid_on_ethereum() {
        // Chain is part of identity; the same string is not interchangeable.
        let btc = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa";
        assert!(parse_address(Chain::Bitcoin, btc, "x").is_ok());
        assert!(parse_address(Chain::Ethereum, btc, "x").is_err());
    }

    #[test]
    fn chains_round_trip_with_aliases() {
        assert_eq!(Chain::parse("BTC"), Some(Chain::Bitcoin));
        assert_eq!(Chain::parse("ethereum"), Some(Chain::Ethereum));
        assert_eq!(Chain::parse("dogecoin"), None);
        for chain in [Chain::Bitcoin, Chain::Ethereum] {
            assert_eq!(Chain::parse(chain.as_str()), Some(chain));
        }
    }

    #[test]
    fn labels_are_trimmed() {
        let parsed =
            parse_address(Chain::Bitcoin, "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa", "  cold  ")
                .unwrap();
        assert_eq!(parsed.label, "cold");
    }
}
