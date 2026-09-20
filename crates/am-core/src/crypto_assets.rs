//! Cryptocurrency holdings.
//!
//! # Why identity is a stable ID, never a symbol
//!
//! A ticker symbol is not an identifier. There are several tokens called
//! "UNI", more than one "BTC" wrapper, and any number of scam tokens that
//! deliberately reuse a famous symbol. Pricing a holding by symbol means
//! occasionally pricing the wrong asset — and on a portfolio screen that
//! error is invisible, because the number still looks plausible.
//!
//! So a holding stores a **provider-stable coin ID** (`bitcoin`,
//! `ethereum`) and, for tokens, the chain and contract address that pin down
//! exactly which asset is meant. The symbol is kept for display only.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::money::{Currency, Money, MoneyError};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CryptoError {
    #[error("a coin id is required — a ticker symbol is not enough to identify an asset")]
    MissingCoinId,
    #[error("coin id {0:?} is not in the expected format")]
    BadCoinId(String),
    #[error("quantity cannot be negative")]
    NegativeQuantity,
    #[error(transparent)]
    Money(#[from] MoneyError),
}

/// Where a holding lives. Recorded because "which exchange?" is the first
/// question asked when reconciling a balance, and because self-custody and
/// exchange custody carry different risks worth seeing separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Custody {
    /// A wallet whose keys the owner holds.
    SelfCustody,
    /// Held by an exchange or custodian.
    Exchange,
    /// Staked, lent, or otherwise committed — not immediately spendable.
    Locked,
}

impl Custody {
    pub fn as_str(self) -> &'static str {
        match self {
            Custody::SelfCustody => "self_custody",
            Custody::Exchange => "exchange",
            Custody::Locked => "locked",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "self_custody" => Custody::SelfCustody,
            "exchange" => Custody::Exchange,
            "locked" => Custody::Locked,
            _ => return None,
        })
    }
}

/// Identity of a crypto asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoinIdentity {
    /// Provider-stable ID, e.g. `bitcoin`. This is what gets priced.
    pub coin_id: String,
    /// Display only. Never used for lookup.
    pub symbol: String,
    /// Chain a token lives on, e.g. `ethereum`. `None` for a native coin.
    pub chain: Option<String>,
    /// Contract address, which is the only truly unambiguous token identity.
    pub contract: Option<String>,
}

impl CoinIdentity {
    /// A native coin, identified by its provider ID.
    pub fn native(coin_id: &str, symbol: &str) -> Result<Self, CryptoError> {
        let coin_id = normalize_coin_id(coin_id)?;
        Ok(Self {
            coin_id,
            symbol: symbol.trim().to_ascii_uppercase(),
            chain: None,
            contract: None,
        })
    }

    /// A token, pinned to its chain and contract.
    pub fn token(
        coin_id: &str,
        symbol: &str,
        chain: &str,
        contract: &str,
    ) -> Result<Self, CryptoError> {
        let coin_id = normalize_coin_id(coin_id)?;
        Ok(Self {
            coin_id,
            symbol: symbol.trim().to_ascii_uppercase(),
            chain: Some(chain.trim().to_ascii_lowercase()),
            contract: Some(contract.trim().to_ascii_lowercase()),
        })
    }

    /// Instrument identifier used for quote lookup and storage.
    ///
    /// Namespaced by provider because a coin ID is only meaningful relative to
    /// whoever assigned it — CoinGecko's `bitcoin` is not guaranteed to match
    /// another provider's.
    pub fn instrument_id(&self) -> String {
        format!("coingecko:{}", self.coin_id)
    }

    /// Short label for the UI, disambiguated when a contract is known.
    pub fn display_label(&self) -> String {
        match (&self.chain, &self.contract) {
            (Some(chain), Some(contract)) if contract.len() >= 10 => {
                format!(
                    "{} ({} {}…{})",
                    self.symbol,
                    chain,
                    &contract[..6],
                    &contract[contract.len() - 4..]
                )
            }
            (Some(chain), _) => format!("{} ({chain})", self.symbol),
            _ => self.symbol.clone(),
        }
    }
}

/// Coin IDs are lowercase, hyphen-separated slugs.
///
/// Validated rather than trusted: an ID with a stray space or uppercase
/// character silently fails to match at the provider, which looks like "no
/// price available" rather than "you typed it wrong".
fn normalize_coin_id(raw: &str) -> Result<String, CryptoError> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Err(CryptoError::MissingCoinId);
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(CryptoError::BadCoinId(raw.to_string()));
    }
    Ok(trimmed)
}

/// A crypto holding.
#[derive(Debug, Clone)]
pub struct CryptoHolding {
    pub identity: CoinIdentity,
    /// Up to 18 decimal places for an ERC-20 balance. Exact decimal, never a
    /// float: `0.1 + 0.2` must equal `0.3`.
    pub quantity: Decimal,
    pub custody: Custody,
    /// Exchange or wallet name, for reconciliation.
    pub location: Option<String>,
}

impl CryptoHolding {
    /// Value at a unit price.
    ///
    /// Rounds once, at the end. A price like $0.000012 multiplied by a large
    /// balance is exactly the case where rounding the unit price first would
    /// erase the position.
    pub fn value(&self, unit_price: Decimal, currency: Currency) -> Result<Money, CryptoError> {
        if self.quantity.is_sign_negative() {
            return Err(CryptoError::NegativeQuantity);
        }
        Ok(Money::from_total_decimal(self.quantity * unit_price, currency)?)
    }
}

/// Well-known coins, so common holdings can be added without looking up an ID.
///
/// Deliberately short. A long bundled list would go stale, and the point is to
/// cover the handful people actually hold while making clear that anything
/// else needs its real coin ID rather than a guessed symbol.
pub const COMMON_COINS: &[(&str, &str, &str)] = &[
    // (coin_id, symbol, display name)
    ("bitcoin", "BTC", "Bitcoin"),
    ("ethereum", "ETH", "Ethereum"),
    ("tether", "USDT", "Tether"),
    ("usd-coin", "USDC", "USD Coin"),
    ("solana", "SOL", "Solana"),
    ("cardano", "ADA", "Cardano"),
    ("ripple", "XRP", "XRP"),
    ("dogecoin", "DOGE", "Dogecoin"),
    ("litecoin", "LTC", "Litecoin"),
    ("monero", "XMR", "Monero"),
    ("chainlink", "LINK", "Chainlink"),
    ("polkadot", "DOT", "Polkadot"),
];

pub fn common_coin(coin_id: &str) -> Option<(&'static str, &'static str, &'static str)> {
    COMMON_COINS.iter().copied().find(|(id, _, _)| *id == coin_id)
}

/// Look up by symbol, reporting ambiguity rather than guessing.
///
/// Returns every match. A caller must not silently take the first: that is
/// precisely how a holding gets priced as the wrong asset.
pub fn coins_by_symbol(symbol: &str) -> Vec<(&'static str, &'static str, &'static str)> {
    let wanted = symbol.trim().to_ascii_uppercase();
    COMMON_COINS.iter().copied().filter(|(_, sym, _)| *sym == wanted).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::money::parse_decimal;

    fn usd() -> Currency {
        Currency::new("USD").unwrap()
    }

    fn d(s: &str) -> Decimal {
        parse_decimal(s).unwrap()
    }

    #[test]
    fn coin_ids_are_normalized_and_validated() {
        let btc = CoinIdentity::native("Bitcoin", "btc").unwrap();
        assert_eq!(btc.coin_id, "bitcoin", "ids are lowercased");
        assert_eq!(btc.symbol, "BTC", "symbols are uppercased for display");

        assert_eq!(CoinIdentity::native("", "BTC"), Err(CryptoError::MissingCoinId));
        // A stray space would fail silently at the provider, so catch it here.
        assert!(matches!(
            CoinIdentity::native("bit coin", "BTC"),
            Err(CryptoError::BadCoinId(_))
        ));
    }

    #[test]
    fn instrument_ids_are_namespaced_by_provider() {
        // A coin id is only meaningful relative to whoever assigned it.
        let btc = CoinIdentity::native("bitcoin", "BTC").unwrap();
        assert_eq!(btc.instrument_id(), "coingecko:bitcoin");
    }

    #[test]
    fn tokens_record_chain_and_contract() {
        let usdc = CoinIdentity::token(
            "usd-coin",
            "usdc",
            "Ethereum",
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
        )
        .unwrap();

        assert_eq!(usdc.chain.as_deref(), Some("ethereum"));
        assert_eq!(
            usdc.contract.as_deref(),
            Some("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"),
            "contracts are lowercased so comparison is case-insensitive"
        );
    }

    #[test]
    fn a_token_label_shows_enough_to_tell_two_apart() {
        // Two tokens sharing a symbol must be distinguishable at a glance.
        let real = CoinIdentity::token(
            "usd-coin",
            "USDC",
            "ethereum",
            "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
        )
        .unwrap();
        let impostor = CoinIdentity::token(
            "fake-usdc",
            "USDC",
            "ethereum",
            "0xdeadbeef00000000000000000000000000001234",
        )
        .unwrap();

        assert_ne!(real.display_label(), impostor.display_label());
        assert!(real.display_label().contains("USDC"));
    }

    #[test]
    fn a_native_coin_label_is_just_its_symbol() {
        let btc = CoinIdentity::native("bitcoin", "BTC").unwrap();
        assert_eq!(btc.display_label(), "BTC");
    }

    #[test]
    fn symbol_lookup_reports_matches_rather_than_guessing() {
        let matches = coins_by_symbol("btc");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, "bitcoin");

        assert!(coins_by_symbol("NOTACOIN").is_empty(), "no match, not a wrong guess");
    }

    #[test]
    fn eighteen_decimal_balances_stay_exact() {
        let holding = CryptoHolding {
            identity: CoinIdentity::native("ethereum", "ETH").unwrap(),
            quantity: d("1.234567890123456789"),
            custody: Custody::SelfCustody,
            location: None,
        };
        assert_eq!(holding.quantity.to_string(), "1.234567890123456789");
    }

    #[test]
    fn a_sub_cent_price_does_not_erase_a_large_position() {
        // The failure this rounding discipline exists to prevent.
        let holding = CryptoHolding {
            identity: CoinIdentity::native("some-token", "TOK").unwrap(),
            quantity: Decimal::from(100_000),
            custody: Custody::Exchange,
            location: Some("Example Exchange".into()),
        };
        assert_eq!(holding.value(d("0.004"), usd()).unwrap().format(), "400.00 USD");

        // And an even smaller price on a bigger balance.
        let dust = CryptoHolding { quantity: Decimal::from(1_000_000), ..holding.clone() };
        assert_eq!(dust.value(d("0.0000123"), usd()).unwrap().format(), "12.30 USD");
    }

    #[test]
    fn a_realistic_bitcoin_holding_values_correctly() {
        let holding = CryptoHolding {
            identity: CoinIdentity::native("bitcoin", "BTC").unwrap(),
            quantity: d("0.12345678"),
            custody: Custody::SelfCustody,
            location: Some("hardware wallet".into()),
        };
        // 0.12345678 × 67432.19 = 8324.9610457482, rounding to 8324.96.
        assert_eq!(holding.value(d("67432.19"), usd()).unwrap().format(), "8324.96 USD");
    }

    #[test]
    fn negative_quantities_are_refused() {
        let holding = CryptoHolding {
            identity: CoinIdentity::native("bitcoin", "BTC").unwrap(),
            quantity: Decimal::from(-1),
            custody: Custody::SelfCustody,
            location: None,
        };
        assert_eq!(holding.value(d("50000"), usd()), Err(CryptoError::NegativeQuantity));
    }

    #[test]
    fn custody_round_trips() {
        for custody in [Custody::SelfCustody, Custody::Exchange, Custody::Locked] {
            assert_eq!(Custody::parse(custody.as_str()), Some(custody));
        }
        assert_eq!(Custody::parse("cold_storage"), None);
    }

    #[test]
    fn common_coins_have_valid_identities() {
        // A typo in the bundled list would surface as an unpriceable holding.
        for (coin_id, symbol, _name) in COMMON_COINS {
            let identity = CoinIdentity::native(coin_id, symbol)
                .unwrap_or_else(|e| panic!("bundled coin {coin_id} is invalid: {e}"));
            assert_eq!(&identity.coin_id, coin_id, "bundled ids must already be normalized");
        }
    }

    #[test]
    fn bundled_symbols_are_unique() {
        // If two bundled coins shared a symbol, symbol lookup would return
        // both — which is correct behaviour, but the bundled list should not
        // be the source of that ambiguity.
        let mut symbols: Vec<&str> = COMMON_COINS.iter().map(|(_, s, _)| *s).collect();
        symbols.sort_unstable();
        let before = symbols.len();
        symbols.dedup();
        assert_eq!(symbols.len(), before, "a bundled symbol is duplicated");
    }
}
