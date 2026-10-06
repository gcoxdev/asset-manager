//! Holding valuation.
//!
//! Pure functions: no network, no database. A quote comes in, a value comes
//! out, and every step is testable against arithmetic you can check by hand.
//!
//! # The two distinctions that cause silent errors
//!
//! **Unit value vs. holding value.** A quote prices *one unit*; a holding is
//! `quantity` of them. Conflating the two is off by a factor of the quantity
//! and looks plausible on screen.
//!
//! **Gross weight vs. fine metal content.** A 1 oz Gold Eagle weighs 1.0909
//! troy oz gross but contains exactly 1 troy oz of gold (0.9167 fine).
//! Multiplying purity into a weight that *already* expresses fine content
//! discounts it twice — a silent ~8% undervaluation on every coin.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::money::{Currency, Money, MoneyError};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ValuationError {
    #[error("quantity cannot be negative")]
    NegativeQuantity,
    #[error("purity must be between 0 and 1, got {0}")]
    BadPurity(Decimal),
    #[error("weight cannot be negative")]
    NegativeWeight,
    #[error("contradictory inputs: {0}")]
    Contradictory(String),
    #[error("valuation exceeds supported numeric range")]
    Overflow,
    #[error(transparent)]
    Money(#[from] MoneyError),
}

/// Weight units used in precious metals. Conversions are exact, not
/// approximate: a troy ounce is defined as exactly 31.1034768 grams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeightUnit {
    TroyOunce,
    Gram,
    /// 1/20 troy ounce. Common in jewelry.
    Pennyweight,
    /// Avoirdupois ounce — the *kitchen* ounce. Distinct from a troy ounce,
    /// and confusing them overstates a holding by about 9.7%.
    Ounce,
}

impl WeightUnit {
    /// Grams per unit, exact.
    pub fn grams_per_unit(self) -> Decimal {
        use std::str::FromStr;
        match self {
            WeightUnit::TroyOunce => Decimal::from_str("31.1034768").unwrap(),
            WeightUnit::Gram => Decimal::ONE,
            WeightUnit::Pennyweight => Decimal::from_str("1.55517384").unwrap(),
            WeightUnit::Ounce => Decimal::from_str("28.349523125").unwrap(),
        }
    }

    pub fn convert(self, amount: Decimal, to: WeightUnit) -> Result<Decimal, ValuationError> {
        if self == to {
            return Ok(amount);
        }
        amount
            .checked_mul(self.grams_per_unit())
            .and_then(|v| v.checked_div(to.grams_per_unit()))
            .ok_or(ValuationError::Overflow)
    }
}

/// How a weight figure should be interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeightBasis {
    /// Total weight of the item, alloy included. Purity applies.
    Gross,
    /// Weight of the precious metal only. Purity has already been applied and
    /// must NOT be applied again.
    Fine,
}

/// A precious-metal holding.
#[derive(Debug, Clone)]
pub struct MetalHolding {
    /// Number of items — coins, bars, rounds.
    pub quantity: Decimal,
    /// Weight of ONE item.
    pub weight_per_item: Decimal,
    pub weight_unit: WeightUnit,
    pub weight_basis: WeightBasis,
    /// Fineness as a fraction: 0.999, 0.9167, 0.900. Ignored for
    /// [`WeightBasis::Fine`].
    pub purity: Decimal,
}

impl MetalHolding {
    /// Total fine metal content, in the requested unit.
    pub fn fine_weight(&self, unit: WeightUnit) -> Result<Decimal, ValuationError> {
        if self.quantity.is_sign_negative() {
            return Err(ValuationError::NegativeQuantity);
        }
        if self.weight_per_item.is_sign_negative() {
            return Err(ValuationError::NegativeWeight);
        }
        if self.purity <= Decimal::ZERO || self.purity > Decimal::ONE {
            return Err(ValuationError::BadPurity(self.purity));
        }

        let total_weight =
            self.quantity.checked_mul(self.weight_per_item).ok_or(ValuationError::Overflow)?;
        let fine = match self.weight_basis {
            // Purity applied exactly once.
            WeightBasis::Gross => {
                total_weight.checked_mul(self.purity).ok_or(ValuationError::Overflow)?
            }
            WeightBasis::Fine => total_weight,
        };
        self.weight_unit.convert(fine, unit)
    }

    /// Melt value: fine content times the spot price, with no premium.
    pub fn melt_value(
        &self,
        spot_per_troy_oz: Decimal,
        currency: Currency,
    ) -> Result<Money, ValuationError> {
        let fine_oz = self.fine_weight(WeightUnit::TroyOunce)?;
        // Rounded once, at the end.
        Ok(Money::from_total_decimal(
            fine_oz.checked_mul(spot_per_troy_oz).ok_or(ValuationError::Overflow)?,
            currency,
        )?)
    }

    /// Market value: melt plus a premium over spot.
    ///
    /// `premium_pct` is a fraction — 0.05 means 5% over melt. Numismatic coins
    /// carry substantial premiums, so melt alone understates them.
    pub fn market_value(
        &self,
        spot_per_troy_oz: Decimal,
        premium_pct: Decimal,
        currency: Currency,
    ) -> Result<Money, ValuationError> {
        let fine_oz = self.fine_weight(WeightUnit::TroyOunce)?;
        let total = Decimal::ONE
            .checked_add(premium_pct)
            .and_then(|premium| fine_oz.checked_mul(spot_per_troy_oz)?.checked_mul(premium))
            .ok_or(ValuationError::Overflow)?;
        Ok(Money::from_total_decimal(total, currency)?)
    }
}

/// A fungible holding priced per unit — crypto, shares.
#[derive(Debug, Clone)]
pub struct UnitHolding {
    pub quantity: Decimal,
    pub unit_quote: Decimal,
}

impl UnitHolding {
    pub fn value(&self, currency: Currency) -> Result<Money, ValuationError> {
        if self.quantity.is_sign_negative() {
            return Err(ValuationError::NegativeQuantity);
        }
        // quantity × unit_quote in full precision, rounded once.
        Ok(Money::from_total_decimal(
            self.quantity.checked_mul(self.unit_quote).ok_or(ValuationError::Overflow)?,
            currency,
        )?)
    }
}

/// Face value of US 90% silver coinage ("junk silver").
///
/// The convention: $1 face contains 0.715 troy oz of silver. This is the
/// circulated figure — worn coins lost weight — and is what dealers quote.
/// Uncirculated is 0.7234.
pub fn junk_silver_fine_oz(face_value_dollars: Decimal, circulated: bool) -> Decimal {
    use std::str::FromStr;
    let per_dollar = if circulated {
        Decimal::from_str("0.715").unwrap()
    } else {
        Decimal::from_str("0.7234").unwrap()
    };
    face_value_dollars * per_dollar
}

/// Unrealized gain: current value minus acquisition cost.
///
/// Returns `None` when acquisition cost is unknown, rather than assuming zero
/// — which would report the entire current value as profit.
pub fn unrealized_gain(
    current: &Money,
    acquired: Option<&Money>,
) -> Result<Option<Money>, ValuationError> {
    let Some(acquired) = acquired else { return Ok(None) };
    Ok(Some(current.checked_sub(acquired)?))
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
    fn out_of_range_holdings_return_errors_without_panicking() {
        let metal = MetalHolding {
            quantity: Decimal::from(2),
            weight_per_item: Decimal::MAX,
            weight_unit: WeightUnit::TroyOunce,
            weight_basis: WeightBasis::Fine,
            purity: Decimal::ONE,
        };
        assert_eq!(metal.fine_weight(WeightUnit::TroyOunce), Err(ValuationError::Overflow));
        assert_eq!(
            WeightUnit::TroyOunce.convert(Decimal::MAX, WeightUnit::Gram),
            Err(ValuationError::Overflow)
        );
        let metal =
            MetalHolding { quantity: Decimal::ONE, weight_per_item: Decimal::ONE, ..metal };
        assert!(metal.market_value(Decimal::MAX, Decimal::MAX, usd()).is_err());
        assert_eq!(
            UnitHolding { quantity: Decimal::from(2), unit_quote: Decimal::MAX }.value(usd()),
            Err(ValuationError::Overflow)
        );
    }

    #[test]
    fn troy_ounce_is_not_an_ounce() {
        // Confusing the two overstates a holding by ~9.7%.
        let one_troy = WeightUnit::TroyOunce.convert(Decimal::ONE, WeightUnit::Gram).unwrap();
        let one_avoir = WeightUnit::Ounce.convert(Decimal::ONE, WeightUnit::Gram).unwrap();

        assert_eq!(one_troy, d("31.1034768"));
        assert_eq!(one_avoir, d("28.349523125"));
        assert!(one_troy > one_avoir);
    }

    #[test]
    fn weight_conversions_round_trip() {
        for unit in [WeightUnit::Gram, WeightUnit::Pennyweight, WeightUnit::Ounce] {
            let grams = WeightUnit::TroyOunce.convert(Decimal::ONE, unit).unwrap();
            let back = unit.convert(grams, WeightUnit::TroyOunce).unwrap();
            assert_eq!(back.round_dp(10), Decimal::ONE, "round trip via {unit:?}");
        }
    }

    #[test]
    fn twenty_pennyweight_make_a_troy_ounce() {
        let one_oz =
            WeightUnit::Pennyweight.convert(Decimal::from(20), WeightUnit::TroyOunce).unwrap();
        assert_eq!(one_oz.round_dp(10), Decimal::ONE);
    }

    #[test]
    fn gross_weight_applies_purity_exactly_once() {
        // A 1 oz Gold Eagle: 1.0909 troy oz gross, 0.9167 fine, containing
        // exactly 1 troy oz of gold.
        let eagle = MetalHolding {
            quantity: Decimal::ONE,
            weight_per_item: d("1.0909"),
            weight_unit: WeightUnit::TroyOunce,
            weight_basis: WeightBasis::Gross,
            purity: d("0.9167"),
        };

        let fine = eagle.fine_weight(WeightUnit::TroyOunce).unwrap();
        assert_eq!(fine.round_dp(3), Decimal::ONE, "should contain 1 troy oz of gold");
    }

    #[test]
    fn fine_basis_does_not_reapply_purity() {
        // The silent ~8% error: the same coin described by its fine content.
        // Applying purity again would give 0.9167 oz instead of 1.
        let by_fine = MetalHolding {
            quantity: Decimal::ONE,
            weight_per_item: Decimal::ONE,
            weight_unit: WeightUnit::TroyOunce,
            weight_basis: WeightBasis::Fine,
            purity: d("0.9167"), // present, and must be ignored
        };

        assert_eq!(by_fine.fine_weight(WeightUnit::TroyOunce).unwrap(), Decimal::ONE);
    }

    #[test]
    fn quantity_multiplies_per_item_weight() {
        // The unit-vs-holding distinction: ten coins, not one.
        let tube = MetalHolding {
            quantity: Decimal::from(10),
            weight_per_item: Decimal::ONE,
            weight_unit: WeightUnit::TroyOunce,
            weight_basis: WeightBasis::Fine,
            purity: Decimal::ONE,
        };
        assert_eq!(tube.fine_weight(WeightUnit::TroyOunce).unwrap(), Decimal::from(10));

        let value = tube.melt_value(d("2000"), usd()).unwrap();
        assert_eq!(value.format(), "20000.00 USD", "ten ounces, not one");
    }

    #[test]
    fn melt_value_of_a_known_holding() {
        // 10 oz bar, .999 fine, spot $30/oz -> 9.99 oz × $30 = $299.70
        let bar = MetalHolding {
            quantity: Decimal::ONE,
            weight_per_item: Decimal::from(10),
            weight_unit: WeightUnit::TroyOunce,
            weight_basis: WeightBasis::Gross,
            purity: d("0.999"),
        };
        assert_eq!(bar.melt_value(d("30"), usd()).unwrap().format(), "299.70 USD");
    }

    #[test]
    fn premium_is_applied_over_melt() {
        let coin = MetalHolding {
            quantity: Decimal::ONE,
            weight_per_item: Decimal::ONE,
            weight_unit: WeightUnit::TroyOunce,
            weight_basis: WeightBasis::Fine,
            purity: Decimal::ONE,
        };

        let melt = coin.melt_value(d("2000"), usd()).unwrap();
        let market = coin.market_value(d("2000"), d("0.05"), usd()).unwrap();

        assert_eq!(melt.format(), "2000.00 USD");
        assert_eq!(market.format(), "2100.00 USD", "5% premium over melt");
    }

    #[test]
    fn grams_and_pennyweight_holdings_value_correctly() {
        // 100 g of .999 silver at $30/troy oz.
        let bar = MetalHolding {
            quantity: Decimal::ONE,
            weight_per_item: Decimal::from(100),
            weight_unit: WeightUnit::Gram,
            weight_basis: WeightBasis::Gross,
            purity: d("0.999"),
        };
        let fine_oz = bar.fine_weight(WeightUnit::TroyOunce).unwrap();
        // 100 g × 0.999 / 31.1034768 = 3.211859582... troy oz, which rounds
        // to 3.2119 at 4dp. (An earlier expectation of 3.2118 truncated
        // rather than rounded.)
        assert_eq!(fine_oz.round_dp(4), d("3.2119"));
        assert_eq!(bar.melt_value(d("30"), usd()).unwrap().format(), "96.36 USD");
    }

    #[test]
    fn junk_silver_uses_the_face_value_convention() {
        // $1 face of 90% silver, circulated.
        assert_eq!(junk_silver_fine_oz(Decimal::ONE, true), d("0.715"));
        // A $10 face bag.
        assert_eq!(junk_silver_fine_oz(Decimal::from(10), true), d("7.150"));
        // Uncirculated is slightly heavier.
        assert_eq!(junk_silver_fine_oz(Decimal::ONE, false), d("0.7234"));
    }

    #[test]
    fn crypto_quantities_keep_full_precision() {
        let holding = UnitHolding { quantity: d("0.12345678"), unit_quote: d("67432.19") };
        // 0.12345678 × 67432.19 = 8324.9610457482 exactly (verified
        // independently), so it rounds to 8324.96.
        assert_eq!(holding.value(usd()).unwrap().format(), "8324.96 USD");
    }

    #[test]
    fn sub_cent_unit_quotes_do_not_zero_out() {
        // The $400 case again, at the valuation layer.
        let holding = UnitHolding { quantity: Decimal::from(100_000), unit_quote: d("0.004") };
        assert_eq!(holding.value(usd()).unwrap().format(), "400.00 USD");
    }

    #[test]
    fn negative_quantities_are_rejected() {
        let holding = UnitHolding { quantity: Decimal::from(-5), unit_quote: Decimal::ONE };
        assert_eq!(holding.value(usd()), Err(ValuationError::NegativeQuantity));
    }

    #[test]
    fn impossible_purity_is_rejected() {
        let bad = MetalHolding {
            quantity: Decimal::ONE,
            weight_per_item: Decimal::ONE,
            weight_unit: WeightUnit::TroyOunce,
            weight_basis: WeightBasis::Gross,
            purity: Decimal::from(2), // 200% pure
        };
        assert!(matches!(
            bad.fine_weight(WeightUnit::TroyOunce),
            Err(ValuationError::BadPurity(_))
        ));

        let zero = MetalHolding { purity: Decimal::ZERO, ..bad };
        assert!(matches!(
            zero.fine_weight(WeightUnit::TroyOunce),
            Err(ValuationError::BadPurity(_))
        ));
    }

    #[test]
    fn unknown_acquisition_cost_yields_no_gain_not_a_fake_one() {
        let current = Money::new(1_900_000, usd());
        assert_eq!(unrealized_gain(&current, None).unwrap(), None);

        let acquired = Money::new(1_200_000, usd());
        let gain = unrealized_gain(&current, Some(&acquired)).unwrap().unwrap();
        assert_eq!(gain.format(), "7000.00 USD");
    }

    #[test]
    fn a_loss_is_reported_as_negative() {
        let current = Money::new(80_000, usd());
        let acquired = Money::new(100_000, usd());
        let gain = unrealized_gain(&current, Some(&acquired)).unwrap().unwrap();
        assert_eq!(gain.format(), "-200.00 USD");
    }

    #[test]
    fn gain_across_currencies_is_refused() {
        let current = Money::new(100, usd());
        let acquired = Money::new(100, Currency::new("EUR").unwrap());
        assert!(unrealized_gain(&current, Some(&acquired)).is_err());
    }

    #[test]
    fn a_realistic_portfolio_totals_correctly() {
        // Hand-checkable: 10 Silver Eagles + 1 Gold Eagle + 0.5 BTC.
        let eagles = MetalHolding {
            quantity: Decimal::from(10),
            weight_per_item: Decimal::ONE,
            weight_unit: WeightUnit::TroyOunce,
            weight_basis: WeightBasis::Fine,
            purity: d("0.999"),
        };
        let gold = MetalHolding {
            quantity: Decimal::ONE,
            weight_per_item: d("1.0909"),
            weight_unit: WeightUnit::TroyOunce,
            weight_basis: WeightBasis::Gross,
            purity: d("0.9167"),
        };
        let btc = UnitHolding { quantity: d("0.5"), unit_quote: d("67000") };

        let silver_value = eagles.melt_value(d("30"), usd()).unwrap(); // 10 oz × 30 = 300
        let gold_value = gold.melt_value(d("2000"), usd()).unwrap(); // ~1 oz × 2000
        let btc_value = btc.value(usd()).unwrap(); // 33500

        assert_eq!(silver_value.format(), "300.00 USD");
        // 1.0909 × 0.9167 = 1.00002803 fine oz, so slightly over $2000.
        assert_eq!(gold_value.format(), "2000.06 USD");
        assert_eq!(btc_value.format(), "33500.00 USD");

        let total =
            silver_value.checked_add(&gold_value).unwrap().checked_add(&btc_value).unwrap();
        assert_eq!(total.format(), "35800.06 USD");
    }
}
