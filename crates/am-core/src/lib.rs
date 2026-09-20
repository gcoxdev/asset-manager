//! Domain logic: money, quantities, and valuation.
//!
//! Pure functions with no I/O, so the arithmetic that decides what a
//! collection is worth can be tested against figures you can check by hand.

pub mod collectibles;
pub mod crypto_assets;
pub mod metals;
pub mod money;
pub mod valuation;
pub mod watch_only;

pub use collectibles::{
    collectible_type, describe, validate, Attributes, CollectibleError, CollectibleType,
    Grader, COLLECTIBLE_TYPES,
};
pub use crypto_assets::{
    coins_by_symbol, CoinIdentity, CryptoError, CryptoHolding, Custody, COMMON_COINS,
};
pub use metals::{preset, BullionPreset, Freshness, Metal, QuotaBudget, PRESETS};
pub use money::{parse_decimal, sort_key, Currency, Money, MoneyError};
pub use rust_decimal::Decimal;
pub use valuation::{
    junk_silver_fine_oz, unrealized_gain, MetalHolding, UnitHolding, ValuationError,
    WeightBasis, WeightUnit,
};
pub use watch_only::{parse_address, Chain, WatchAddress, WatchOnlyError};
