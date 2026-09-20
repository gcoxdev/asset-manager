//! Money and exact-decimal quantities.
//!
//! Two representations, deliberately distinct types so they cannot be mixed up:
//!
//! - [`Money`] — a *finalized* fiat amount: integer minor units plus a
//!   currency. This is what gets stored and displayed.
//! - [`rust_decimal::Decimal`] — everything else: unit quotes, quantities,
//!   weights, purities, premiums, FX rates.
//!
//! # The mistake this design prevents
//!
//! Rounding a *unit quote* to minor units destroys value:
//!
//! ```text
//! 100,000 units × $0.004 = $400.00        correct
//! 100,000 units × $0.00   = $0.00         quote rounded to cents first
//! ```
//!
//! The entire position disappears. So [`Money`] cannot be constructed from a
//! per-unit price at all — only from a *total*, via
//! [`Money::from_total_decimal`], which rounds once at the end.

use std::fmt;

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MoneyError {
    #[error("currency code must be three ASCII letters, got {0:?}")]
    BadCurrency(String),
    #[error("amount is too large to represent")]
    Overflow,
    #[error("mismatched currencies: {left} and {right}")]
    CurrencyMismatch { left: String, right: String },
    #[error("{0:?} is not a valid decimal number")]
    NotDecimal(String),
}

/// ISO 4217 code, uppercase. Not an enum: a user may hold something we did
/// not anticipate, and refusing to record it would be worse than accepting a
/// code we cannot convert.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Currency(String);

impl Currency {
    pub fn new(code: &str) -> Result<Self, MoneyError> {
        let upper = code.trim().to_ascii_uppercase();
        if upper.len() != 3 || !upper.bytes().all(|b| b.is_ascii_alphabetic()) {
            return Err(MoneyError::BadCurrency(code.to_string()));
        }
        Ok(Self(upper))
    }

    pub fn code(&self) -> &str {
        &self.0
    }

    /// Digits after the decimal point.
    ///
    /// Most currencies use 2, but not all — JPY has none, and treating it as
    /// 2 would display ¥1,000 as ¥10.00. The zero-decimal list is the common
    /// subset; anything unlisted defaults to 2.
    pub fn minor_digits(&self) -> u32 {
        const ZERO_DECIMAL: &[&str] =
            &["JPY", "KRW", "VND", "CLP", "ISK", "HUF", "TWD", "UGX", "XAF", "XOF", "XPF"];
        const THREE_DECIMAL: &[&str] = &["BHD", "IQD", "JOD", "KWD", "LYD", "OMR", "TND"];

        if ZERO_DECIMAL.contains(&self.0.as_str()) {
            0
        } else if THREE_DECIMAL.contains(&self.0.as_str()) {
            3
        } else {
            2
        }
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A finalized amount: integer minor units in a specific currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    /// Cents, pence, satoshi-of-fiat. Never a float.
    pub amount_minor: i64,
    pub currency: Currency,
}

impl Money {
    pub fn new(amount_minor: i64, currency: Currency) -> Self {
        Self { amount_minor, currency }
    }

    pub fn zero(currency: Currency) -> Self {
        Self { amount_minor: 0, currency }
    }

    /// Build from a **total** major-unit amount, rounding once.
    ///
    /// Banker's rounding (half-to-even): repeated half-up rounding across many
    /// holdings biases a portfolio total upward.
    ///
    /// Note this takes a *total*, never a per-unit price — see the module
    /// docs for why that distinction is load-bearing.
    pub fn from_total_decimal(total: Decimal, currency: Currency) -> Result<Self, MoneyError> {
        let scale = currency.minor_digits();
        let scaled = total
            .checked_mul(Decimal::from(10_i64.pow(scale)))
            .ok_or(MoneyError::Overflow)?
            .round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven);

        let amount_minor = scaled.to_i64().ok_or(MoneyError::Overflow)?;
        Ok(Self { amount_minor, currency })
    }

    /// Exact major-unit value, for further arithmetic. Never lossy.
    pub fn to_decimal(&self) -> Decimal {
        Decimal::from(self.amount_minor) / Decimal::from(10_i64.pow(self.currency.minor_digits()))
    }

    pub fn checked_add(&self, other: &Money) -> Result<Money, MoneyError> {
        self.require_same_currency(other)?;
        let amount_minor =
            self.amount_minor.checked_add(other.amount_minor).ok_or(MoneyError::Overflow)?;
        Ok(Money { amount_minor, currency: self.currency.clone() })
    }

    pub fn checked_sub(&self, other: &Money) -> Result<Money, MoneyError> {
        self.require_same_currency(other)?;
        let amount_minor =
            self.amount_minor.checked_sub(other.amount_minor).ok_or(MoneyError::Overflow)?;
        Ok(Money { amount_minor, currency: self.currency.clone() })
    }

    /// Adding USD to EUR is always a bug; it is rejected rather than coerced.
    fn require_same_currency(&self, other: &Money) -> Result<(), MoneyError> {
        if self.currency != other.currency {
            return Err(MoneyError::CurrencyMismatch {
                left: self.currency.to_string(),
                right: other.currency.to_string(),
            });
        }
        Ok(())
    }

    /// Format for display, e.g. `1234.50 USD`.
    ///
    /// Built by string manipulation from the integer, so no float ever touches
    /// a displayed amount.
    pub fn format(&self) -> String {
        let digits = self.currency.minor_digits() as usize;
        let negative = self.amount_minor < 0;
        let magnitude = self.amount_minor.unsigned_abs().to_string();

        let body = if digits == 0 {
            magnitude
        } else {
            let padded = format!("{magnitude:0>width$}", width = digits + 1);
            let split = padded.len() - digits;
            format!("{}.{}", &padded[..split], &padded[split..])
        };

        format!("{}{} {}", if negative { "-" } else { "" }, body, self.currency)
    }
}

/// Parse an exact decimal from text.
///
/// Used for every non-money number crossing a boundary — IPC, CSV, the
/// database — because those all carry decimals as strings. Going via `f64`
/// would silently lose precision on values like `0.1`.
pub fn parse_decimal(text: &str) -> Result<Decimal, MoneyError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(MoneyError::NotDecimal(text.to_string()));
    }
    trimmed
        .parse::<Decimal>()
        .map_err(|_| MoneyError::NotDecimal(text.to_string()))
}

/// A sort key for a decimal stored as TEXT.
///
/// SQLite sorts TEXT lexicographically, so `"9" > "10"`. A `REAL` companion
/// column fixes ordering. It is **only** ever used for `ORDER BY` — never for
/// arithmetic or display, where its precision loss would matter.
pub fn sort_key(value: Decimal) -> f64 {
    value.to_f64().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn usd() -> Currency {
        Currency::new("USD").unwrap()
    }

    #[test]
    fn currency_codes_are_validated_and_normalized() {
        assert_eq!(Currency::new("usd").unwrap().code(), "USD");
        assert_eq!(Currency::new(" eur ").unwrap().code(), "EUR");

        assert!(Currency::new("US").is_err());
        assert!(Currency::new("USDD").is_err());
        assert!(Currency::new("US1").is_err());
        assert!(Currency::new("").is_err());
    }

    #[test]
    fn minor_digits_are_not_always_two() {
        assert_eq!(usd().minor_digits(), 2);
        assert_eq!(Currency::new("JPY").unwrap().minor_digits(), 0);
        assert_eq!(Currency::new("KWD").unwrap().minor_digits(), 3);
    }

    #[test]
    fn zero_decimal_currencies_format_without_a_point() {
        // ¥1000 is a thousand yen, not ten. Assuming 2 digits would divide by
        // a hundred on screen.
        let jpy = Money::new(1000, Currency::new("JPY").unwrap());
        assert_eq!(jpy.format(), "1000 JPY");
    }

    #[test]
    fn the_four_hundred_dollar_case() {
        // The failure this whole module exists to prevent: 100,000 units at
        // $0.004 is $400, and must never become $0.
        let quantity = Decimal::from(100_000);
        let unit_quote = Decimal::from_str("0.004").unwrap();

        let total = quantity * unit_quote;
        let money = Money::from_total_decimal(total, usd()).unwrap();

        assert_eq!(money.amount_minor, 40_000);
        assert_eq!(money.format(), "400.00 USD");
    }

    #[test]
    fn sub_cent_quotes_survive_multiplication() {
        // Crypto and junk silver both produce quotes below one minor unit.
        for (qty, quote, expected) in [
            ("1000000", "0.00000123", "1.23 USD"),
            ("33", "0.015", "0.50 USD"), // 0.495 -> banker's rounding
            ("0.5", "1999.99", "1000.00 USD"), // half an ounce of gold
        ] {
            let total = parse_decimal(qty).unwrap() * parse_decimal(quote).unwrap();
            let money = Money::from_total_decimal(total, usd()).unwrap();
            assert_eq!(money.format(), expected, "qty {qty} at {quote}");
        }
    }

    #[test]
    fn rounding_is_half_to_even() {
        // Half-up on every holding biases a portfolio total upward.
        let cases = [
            ("0.125", 12), // 12.5 minor -> 12 (even)
            ("0.135", 14), // 13.5 minor -> 14 (even)
            ("0.145", 14), // 14.5 minor -> 14 (even)
        ];
        for (input, expected_minor) in cases {
            let money =
                Money::from_total_decimal(parse_decimal(input).unwrap(), usd()).unwrap();
            assert_eq!(money.amount_minor, expected_minor, "rounding {input}");
        }
    }

    #[test]
    fn money_roundtrips_through_decimal() {
        let original = Money::new(129_900, usd());
        let back = Money::from_total_decimal(original.to_decimal(), usd()).unwrap();
        assert_eq!(original, back);
    }

    #[test]
    fn adding_different_currencies_is_rejected() {
        let dollars = Money::new(100, usd());
        let euros = Money::new(100, Currency::new("EUR").unwrap());

        assert!(matches!(
            dollars.checked_add(&euros),
            Err(MoneyError::CurrencyMismatch { .. })
        ));
    }

    #[test]
    fn overflow_is_reported_not_wrapped() {
        let huge = Money::new(i64::MAX, usd());
        let one = Money::new(1, usd());
        assert_eq!(huge.checked_add(&one), Err(MoneyError::Overflow));

        // And a decimal too large for i64 minor units.
        let enormous = Decimal::from_str("99999999999999999999").unwrap();
        assert_eq!(
            Money::from_total_decimal(enormous, usd()),
            Err(MoneyError::Overflow)
        );
    }

    #[test]
    fn negative_amounts_format_correctly() {
        assert_eq!(Money::new(-1250, usd()).format(), "-12.50 USD");
        assert_eq!(Money::new(-5, usd()).format(), "-0.05 USD");
    }

    #[test]
    fn small_amounts_pad_correctly() {
        assert_eq!(Money::new(0, usd()).format(), "0.00 USD");
        assert_eq!(Money::new(5, usd()).format(), "0.05 USD");
        assert_eq!(Money::new(50, usd()).format(), "0.50 USD");
        assert_eq!(Money::new(100, usd()).format(), "1.00 USD");
    }

    #[test]
    fn decimal_parsing_rejects_junk() {
        assert!(parse_decimal("1.5").is_ok());
        assert!(parse_decimal("-0.001").is_ok());
        assert!(parse_decimal("  2.25  ").is_ok());

        assert!(parse_decimal("").is_err());
        assert!(parse_decimal("abc").is_err());
        assert!(parse_decimal("1.2.3").is_err());
        assert!(parse_decimal("1,000").is_err()); // thousands separators are ambiguous
    }

    #[test]
    fn float_precision_loss_is_avoided() {
        // The canonical float failure: 0.1 + 0.2 != 0.3 in binary floating
        // point. Exact decimals must not reproduce it.
        let a = parse_decimal("0.1").unwrap();
        let b = parse_decimal("0.2").unwrap();
        assert_eq!(a + b, parse_decimal("0.3").unwrap());

        // ...and the f64 path genuinely does fail, so the test is meaningful.
        assert_ne!(0.1_f64 + 0.2_f64, 0.3_f64);
    }

    #[test]
    fn crypto_precision_survives() {
        // 18 decimal places, as an ERC-20 balance would carry.
        let balance = parse_decimal("1.234567890123456789").unwrap();
        assert_eq!(balance.to_string(), "1.234567890123456789");
    }

    #[test]
    fn sort_key_orders_numerically_where_text_would_not() {
        let mut values: Vec<&str> = vec!["9", "10", "100", "0.5"];
        values.sort(); // lexicographic, as SQLite would sort TEXT
        assert_eq!(values, ["0.5", "10", "100", "9"], "text ordering is wrong, as expected");

        let mut keyed: Vec<(&str, f64)> = ["9", "10", "100", "0.5"]
            .iter()
            .map(|v| (*v, sort_key(parse_decimal(v).unwrap())))
            .collect();
        keyed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        assert_eq!(
            keyed.iter().map(|(v, _)| *v).collect::<Vec<_>>(),
            ["0.5", "9", "10", "100"]
        );
    }
}
