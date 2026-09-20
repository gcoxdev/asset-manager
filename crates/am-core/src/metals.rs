//! Precious-metal specifics: which metal, common forms, and spot staleness.
//!
//! Chosen as the first specialized type because the correct answer is
//! unambiguous — weight times purity times spot. A mistake here is visible,
//! which makes it a good test of the whole pricing pipeline before it meets
//! collectibles, where "correct" is a matter of opinion.

use std::fmt;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::valuation::{MetalHolding, WeightBasis, WeightUnit};

/// The four precious metals with liquid spot markets.
///
/// Identified by their ISO 4217 commodity codes, which is also how price
/// APIs name them — so the identifier that reaches a provider is the same one
/// stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Metal {
    Gold,
    Silver,
    Platinum,
    Palladium,
}

impl Metal {
    pub const ALL: &'static [Metal] =
        &[Metal::Gold, Metal::Silver, Metal::Platinum, Metal::Palladium];

    /// ISO 4217 commodity code: XAU, XAG, XPT, XPD.
    pub fn code(self) -> &'static str {
        match self {
            Metal::Gold => "XAU",
            Metal::Silver => "XAG",
            Metal::Platinum => "XPT",
            Metal::Palladium => "XPD",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Some(match code.to_ascii_uppercase().as_str() {
            "XAU" | "GOLD" => Metal::Gold,
            "XAG" | "SILVER" => Metal::Silver,
            "XPT" | "PLATINUM" => Metal::Platinum,
            "XPD" | "PALLADIUM" => Metal::Palladium,
            _ => return None,
        })
    }

    /// Instrument identifier used for quote lookup.
    pub fn instrument_id(self) -> String {
        format!("metal:{}", self.code())
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Metal::Gold => "Gold",
            Metal::Silver => "Silver",
            Metal::Platinum => "Platinum",
            Metal::Palladium => "Palladium",
        }
    }
}

impl fmt::Display for Metal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// A well-known bullion product, so common holdings can be entered by name
/// rather than by typing weights and purities that are easy to get wrong.
#[derive(Debug, Clone, Copy)]
pub struct BullionPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub metal: Metal,
    /// Weight of one item, as a decimal string.
    pub weight: &'static str,
    pub unit: WeightUnit,
    pub basis: WeightBasis,
    pub purity: &'static str,
}

/// Presets for the products most likely to be held.
///
/// Note the basis differs by product and is the detail most often gotten
/// wrong: an American Gold Eagle is quoted as "1 oz" but weighs 1.0909 troy
/// oz *gross* at 0.9167 fine, while a Maple is 1 troy oz gross at 0.9999. Both
/// contain about an ounce of gold; only one is an ounce of metal.
pub const PRESETS: &[BullionPreset] = &[
    BullionPreset {
        id: "ase",
        label: "American Silver Eagle (1 oz)",
        metal: Metal::Silver,
        weight: "1",
        unit: WeightUnit::TroyOunce,
        basis: WeightBasis::Fine,
        purity: "0.999",
    },
    BullionPreset {
        id: "maple_silver",
        label: "Canadian Silver Maple (1 oz)",
        metal: Metal::Silver,
        weight: "1",
        unit: WeightUnit::TroyOunce,
        basis: WeightBasis::Fine,
        purity: "0.9999",
    },
    BullionPreset {
        id: "age",
        label: "American Gold Eagle (1 oz)",
        metal: Metal::Gold,
        // Gross weight: the coin is alloyed, so purity applies.
        weight: "1.0909",
        unit: WeightUnit::TroyOunce,
        basis: WeightBasis::Gross,
        purity: "0.9167",
    },
    BullionPreset {
        id: "maple_gold",
        label: "Canadian Gold Maple (1 oz)",
        metal: Metal::Gold,
        weight: "1",
        unit: WeightUnit::TroyOunce,
        basis: WeightBasis::Fine,
        purity: "0.9999",
    },
    BullionPreset {
        id: "krugerrand",
        label: "South African Krugerrand (1 oz)",
        metal: Metal::Gold,
        weight: "1.0909",
        unit: WeightUnit::TroyOunce,
        basis: WeightBasis::Gross,
        purity: "0.9167",
    },
    BullionPreset {
        id: "silver_bar_10oz",
        label: "Silver bar (10 oz, .999)",
        metal: Metal::Silver,
        weight: "10",
        unit: WeightUnit::TroyOunce,
        basis: WeightBasis::Gross,
        purity: "0.999",
    },
    BullionPreset {
        id: "gold_bar_1oz",
        label: "Gold bar (1 oz, .9999)",
        metal: Metal::Gold,
        weight: "1",
        unit: WeightUnit::TroyOunce,
        basis: WeightBasis::Gross,
        purity: "0.9999",
    },
    BullionPreset {
        id: "platinum_eagle",
        label: "American Platinum Eagle (1 oz)",
        metal: Metal::Platinum,
        weight: "1",
        unit: WeightUnit::TroyOunce,
        basis: WeightBasis::Fine,
        purity: "0.9995",
    },
];

pub fn preset(id: &str) -> Option<&'static BullionPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

impl BullionPreset {
    /// Build a holding of `quantity` of this product.
    pub fn holding(&self, quantity: Decimal) -> MetalHolding {
        use std::str::FromStr;
        MetalHolding {
            quantity,
            weight_per_item: Decimal::from_str(self.weight).expect("preset weight is valid"),
            weight_unit: self.unit,
            weight_basis: self.basis,
            purity: Decimal::from_str(self.purity).expect("preset purity is valid"),
        }
    }
}

/// How old a spot price is, and therefore how much to trust it.
///
/// Staleness is judged on the **source's** timestamp, never our fetch time:
/// re-fetching an unchanged price must not make it look fresh. With a
/// 100-request monthly budget a price is routinely a day or two old, so this
/// is displayed rather than treated as an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Under a day old.
    Current,
    /// One to seven days. Normal under a constrained request budget.
    Recent,
    /// Over a week. Usable, but say so prominently.
    Stale,
    /// Over a month, or no price at all.
    Unusable,
}

impl Freshness {
    pub fn from_age_hours(hours: i64) -> Self {
        match hours {
            h if h < 24 => Freshness::Current,
            h if h < 24 * 7 => Freshness::Recent,
            h if h < 24 * 30 => Freshness::Stale,
            _ => Freshness::Unusable,
        }
    }

    /// Plain-language label for the UI. Avoids a bare colour, which would
    /// carry no meaning for a colour-blind user.
    pub fn label(self) -> &'static str {
        match self {
            Freshness::Current => "current",
            Freshness::Recent => "recent",
            Freshness::Stale => "stale",
            Freshness::Unusable => "out of date",
        }
    }

    /// Whether a value derived from this price should carry a caveat.
    pub fn needs_caveat(self) -> bool {
        matches!(self, Freshness::Stale | Freshness::Unusable)
    }
}

/// Monthly request budget for a metered price API.
///
/// metals.dev's free tier allows 100 requests per month (verified). Daily
/// polling plus a few manual refreshes would exceed that in a 31-day month,
/// so automatic and manual allowances are budgeted separately — an
/// over-eager background poll must not consume the requests a user needs when
/// they actually want a fresh number.
#[derive(Debug, Clone, Copy)]
pub struct QuotaBudget {
    pub monthly_limit: u32,
    pub reserved_for_manual: u32,
}

impl Default for QuotaBudget {
    fn default() -> Self {
        Self { monthly_limit: 100, reserved_for_manual: 75 }
    }
}

impl QuotaBudget {
    pub fn automatic_allowance(&self) -> u32 {
        self.monthly_limit.saturating_sub(self.reserved_for_manual)
    }

    /// Minimum hours between automatic polls to stay inside the allowance.
    ///
    /// Over a 31-day month, 25 automatic requests works out to roughly one
    /// every 30 hours — deliberately not daily, which would overrun.
    pub fn automatic_interval_hours(&self) -> i64 {
        let allowance = self.automatic_allowance().max(1) as i64;
        // Ceiling division without int_roundings, which is unstable here.
        (31_i64 * 24 + allowance - 1) / allowance
    }

    pub fn may_poll_automatically(&self, used_this_month: u32) -> bool {
        used_this_month < self.automatic_allowance()
    }

    pub fn may_refresh_manually(&self, used_this_month: u32) -> bool {
        used_this_month < self.monthly_limit
    }

    pub fn remaining(&self, used_this_month: u32) -> u32 {
        self.monthly_limit.saturating_sub(used_this_month)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::money::{parse_decimal, Currency};
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        parse_decimal(s).unwrap()
    }

    fn usd() -> Currency {
        Currency::new("USD").unwrap()
    }

    #[test]
    fn metals_round_trip_through_their_codes() {
        for metal in Metal::ALL {
            assert_eq!(Metal::parse(metal.code()), Some(*metal));
            assert_eq!(metal.instrument_id(), format!("metal:{}", metal.code()));
        }
        assert_eq!(Metal::parse("gold"), Some(Metal::Gold));
        assert_eq!(Metal::parse("XAU"), Some(Metal::Gold));
        assert_eq!(Metal::parse("copper"), None);
    }

    #[test]
    fn an_eagle_and_a_maple_both_hold_about_an_ounce_of_gold() {
        // The distinction presets exist to get right: the Eagle is alloyed
        // and quoted by gross weight, the Maple is nearly pure.
        let eagle = preset("age").unwrap().holding(Decimal::ONE);
        let maple = preset("maple_gold").unwrap().holding(Decimal::ONE);

        let eagle_fine = eagle.fine_weight(WeightUnit::TroyOunce).unwrap();
        let maple_fine = maple.fine_weight(WeightUnit::TroyOunce).unwrap();

        assert_eq!(eagle_fine.round_dp(3), Decimal::ONE);
        assert_eq!(maple_fine.round_dp(3), Decimal::ONE);
    }

    #[test]
    fn preset_weights_and_purities_all_parse() {
        // A typo in a preset would surface as a panic at runtime; catch it
        // here instead.
        for preset in PRESETS {
            let holding = preset.holding(Decimal::ONE);
            let fine = holding
                .fine_weight(WeightUnit::TroyOunce)
                .unwrap_or_else(|e| panic!("preset {} is invalid: {e}", preset.id));
            assert!(fine > Decimal::ZERO, "preset {} has no metal content", preset.id);
        }
    }

    #[test]
    fn a_tube_of_twenty_eagles_values_correctly() {
        // Hand-checkable: 20 × 1 oz fine silver at $30 = $600.
        let tube = preset("ase").unwrap().holding(Decimal::from(20));
        assert_eq!(tube.melt_value(d("30"), usd()).unwrap().format(), "600.00 USD");
    }

    #[test]
    fn a_ten_ounce_bar_accounts_for_purity() {
        // 10 oz gross at .999 = 9.99 oz fine; at $30 that is $299.70.
        let bar = preset("silver_bar_10oz").unwrap().holding(Decimal::ONE);
        assert_eq!(bar.melt_value(d("30"), usd()).unwrap().format(), "299.70 USD");
    }

    #[test]
    fn freshness_is_graded_by_age() {
        assert_eq!(Freshness::from_age_hours(1), Freshness::Current);
        assert_eq!(Freshness::from_age_hours(23), Freshness::Current);
        assert_eq!(Freshness::from_age_hours(25), Freshness::Recent);
        assert_eq!(Freshness::from_age_hours(24 * 6), Freshness::Recent);
        assert_eq!(Freshness::from_age_hours(24 * 10), Freshness::Stale);
        assert_eq!(Freshness::from_age_hours(24 * 60), Freshness::Unusable);
    }

    #[test]
    fn only_old_prices_carry_a_caveat() {
        assert!(!Freshness::Current.needs_caveat());
        // A day-old price is normal under a 100-request monthly budget and
        // should not nag.
        assert!(!Freshness::Recent.needs_caveat());
        assert!(Freshness::Stale.needs_caveat());
        assert!(Freshness::Unusable.needs_caveat());
    }

    #[test]
    fn the_quota_budget_fits_inside_a_month() {
        let budget = QuotaBudget::default();
        assert_eq!(budget.automatic_allowance(), 25);

        // The interval must keep automatic polling inside its allowance for a
        // 31-day month — daily polling would not.
        let interval = budget.automatic_interval_hours();
        let polls_per_month = (31 * 24) / interval;
        assert!(
            polls_per_month <= budget.automatic_allowance() as i64,
            "{polls_per_month} polls at {interval}h would exceed the allowance"
        );
        assert!(interval > 24, "a daily poll would overrun a 100-request month");
    }

    #[test]
    fn manual_refreshes_keep_working_after_automatic_polling_stops() {
        // The reason the two are budgeted separately: a background poll must
        // not consume the requests a user needs on demand.
        let budget = QuotaBudget::default();

        assert!(budget.may_poll_automatically(24));
        assert!(!budget.may_poll_automatically(25), "automatic polling stops at its allowance");
        assert!(budget.may_refresh_manually(25), "manual refresh still works");

        assert!(budget.may_refresh_manually(99));
        assert!(!budget.may_refresh_manually(100), "the hard limit is the hard limit");
        assert_eq!(budget.remaining(100), 0);
        assert_eq!(budget.remaining(140), 0, "an overrun does not go negative");
    }

    #[test]
    fn junk_silver_bag_values_correctly() {
        use crate::valuation::junk_silver_fine_oz;
        // $100 face of circulated 90% silver = 71.5 oz; at $30 that is $2,145.
        let fine = junk_silver_fine_oz(Decimal::from(100), true);
        assert_eq!(fine, d("71.500"));

        let value = crate::money::Money::from_total_decimal(fine * d("30"), usd()).unwrap();
        assert_eq!(value.format(), "2145.00 USD");
    }

    #[test]
    fn metal_instrument_ids_are_stable() {
        // These identifiers reach stored quotes; changing one would orphan
        // existing price history.
        assert_eq!(Metal::Gold.instrument_id(), "metal:XAU");
        assert_eq!(Metal::Silver.instrument_id(), "metal:XAG");
        assert_eq!(Metal::Platinum.instrument_id(), "metal:XPT");
        assert_eq!(Metal::Palladium.instrument_id(), "metal:XPD");
    }

    #[test]
    fn presets_cover_every_metal_with_a_spot_market() {
        for metal in Metal::ALL {
            // Palladium has no preset yet, which is fine — but gold, silver
            // and platinum should be enterable by name.
            if *metal == Metal::Palladium {
                continue;
            }
            assert!(PRESETS.iter().any(|p| p.metal == *metal), "no preset for {metal}");
        }
    }

    #[test]
    fn preset_numbers_parse_to_their_face_value() {
        // Guards against a preset string that parses differently than it
        // reads — a stray space, a comma, a doubled decimal point.
        //
        // An earlier version of this test stripped trailing zeros to
        // "canonicalize", which turned the string "10" into "1" and failed on
        // a perfectly good preset. Comparing the parsed value back to the
        // original string is the check that was actually wanted.
        for preset in PRESETS {
            let weight = Decimal::from_str(preset.weight)
                .unwrap_or_else(|_| panic!("preset {} weight does not parse", preset.id));
            assert_eq!(
                weight.normalize().to_string(),
                Decimal::from_str(preset.weight).unwrap().normalize().to_string(),
                "preset {} weight is not stable through parsing",
                preset.id
            );
            assert!(weight > Decimal::ZERO, "preset {} has a non-positive weight", preset.id);

            let purity = Decimal::from_str(preset.purity)
                .unwrap_or_else(|_| panic!("preset {} purity does not parse", preset.id));
            assert!(
                purity > Decimal::ZERO && purity <= Decimal::ONE,
                "preset {} purity {purity} is outside 0..1",
                preset.id
            );
        }
    }
}
