//! Domain logic: money, quantities, and valuation.
//!
//! Pure functions with no I/O, so the arithmetic that decides what a
//! collection is worth can be tested against figures you can check by hand.

pub mod money;
pub mod valuation;

pub use money::{parse_decimal, sort_key, Currency, Money, MoneyError};
pub use valuation::{
    junk_silver_fine_oz, unrealized_gain, MetalHolding, UnitHolding, ValuationError, WeightBasis,
    WeightUnit,
};
pub use rust_decimal::Decimal;
