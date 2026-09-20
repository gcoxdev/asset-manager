//! Per-type attributes for collectibles.
//!
//! Comics, trading cards and numismatic coins each need fields the generic
//! asset row cannot hold. They live in the `attrs` JSON column against a
//! **versioned** schema, validated in the backend on every write path — not
//! just in the form, which can be bypassed.
//!
//! # Why grading is modelled rather than free text
//!
//! A comic at 9.8 and one at 9.0 differ in price by multiples. Grade is the
//! single most price-relevant field on most collectibles, and storing it as
//! an arbitrary string means it cannot be compared, sorted, or matched
//! against a price source. So graders and scales are enumerated, and a
//! grade outside its grader's scale is rejected at the point of entry.
//!
//! # What is deliberately not modelled
//!
//! No attempt is made to encode every variant, printing and pedigree. That
//! is a catalogue project, not an inventory one, and a half-complete
//! taxonomy is worse than free text because it implies a rigour it does not
//! have. Anything unmodelled goes in notes.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Bumped when a type's field set changes incompatibly. Stored per asset so
/// an old row stays readable after a schema change.
pub const COLLECTIBLE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CollectibleError {
    #[error("unknown collectible type: {0}")]
    UnknownType(String),
    #[error("{field} is required for {type_id}")]
    MissingField { type_id: String, field: String },
    #[error("unknown grader: {0}")]
    UnknownGrader(String),
    #[error(
        "{grader} grades run {min} to {max} in steps of {step}; {value} is not a valid grade"
    )]
    GradeOutOfScale { grader: String, value: String, min: String, max: String, step: String },
    #[error("{field} must be a number, got {value:?}")]
    NotANumber { field: String, value: String },
    #[error("{field} is longer than {max} characters")]
    TooLong { field: String, max: usize },
}

/// Third-party grading services.
///
/// Each uses its own scale, and a number means nothing without knowing whose
/// it is — PSA 10 and CGC 10 are not interchangeable claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Grader {
    /// Comics, and increasingly cards.
    Cgc,
    /// Comics.
    Cbcs,
    /// Sports and trading cards.
    Psa,
    /// Cards; uses half-point subgrades.
    Bgs,
    /// Cards.
    Sgc,
    /// Coins.
    Pcgs,
    /// Coins.
    Ngc,
    /// Not professionally graded. The owner's opinion, and labelled as such.
    Raw,
}

impl Grader {
    pub const ALL: &'static [Grader] = &[
        Grader::Cgc,
        Grader::Cbcs,
        Grader::Psa,
        Grader::Bgs,
        Grader::Sgc,
        Grader::Pcgs,
        Grader::Ngc,
        Grader::Raw,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Grader::Cgc => "cgc",
            Grader::Cbcs => "cbcs",
            Grader::Psa => "psa",
            Grader::Bgs => "bgs",
            Grader::Sgc => "sgc",
            Grader::Pcgs => "pcgs",
            Grader::Ngc => "ngc",
            Grader::Raw => "raw",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "cgc" => Grader::Cgc,
            "cbcs" => Grader::Cbcs,
            "psa" => Grader::Psa,
            "bgs" | "beckett" => Grader::Bgs,
            "sgc" => Grader::Sgc,
            "pcgs" => Grader::Pcgs,
            "ngc" => Grader::Ngc,
            "raw" | "ungraded" | "" => Grader::Raw,
            _ => return None,
        })
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Grader::Cgc => "CGC",
            Grader::Cbcs => "CBCS",
            Grader::Psa => "PSA",
            Grader::Bgs => "BGS (Beckett)",
            Grader::Sgc => "SGC",
            Grader::Pcgs => "PCGS",
            Grader::Ngc => "NGC",
            Grader::Raw => "Ungraded",
        }
    }

    /// `(minimum, maximum, step)` for this grader's scale.
    ///
    /// Comic and card graders run 0.5–10 in tenths; coin graders use the
    /// Sheldon scale, 1–70 in whole points. Conflating them would accept a
    /// "PSA 65", which does not exist.
    pub fn scale(self) -> (Decimal, Decimal, Decimal) {
        use std::str::FromStr;
        let tenth = Decimal::from_str("0.1").unwrap();
        let half = Decimal::from_str("0.5").unwrap();
        match self {
            Grader::Cgc | Grader::Cbcs | Grader::Psa | Grader::Sgc => {
                (half, Decimal::from(10), tenth)
            }
            // Beckett uses half-point steps.
            Grader::Bgs => (Decimal::ONE, Decimal::from(10), half),
            // Sheldon scale.
            Grader::Pcgs | Grader::Ngc => (Decimal::ONE, Decimal::from(70), Decimal::ONE),
            // An ungraded item has no numeric grade to validate.
            Grader::Raw => (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO),
        }
    }

    /// Whether `grade` is valid on this grader's scale.
    pub fn accepts(self, grade: Decimal) -> bool {
        if self == Grader::Raw {
            return false; // an ungraded item carries no numeric grade
        }
        let (min, max, step) = self.scale();
        if grade < min || grade > max {
            return false;
        }
        // The grade must land on a step boundary: PSA 9.75 is not a grade.
        ((grade - min) % step).is_zero()
    }
}

/// A collectible type and the fields it expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollectibleType {
    pub id: &'static str,
    pub label: &'static str,
    /// Fields that must be present and non-empty.
    pub required: &'static [&'static str],
    /// Optional fields this type understands. Anything else is preserved but
    /// not validated, so an import from a future version does not lose data.
    pub optional: &'static [&'static str],
}

pub const COLLECTIBLE_TYPES: &[CollectibleType] = &[
    CollectibleType {
        id: "comic",
        label: "Comic book",
        required: &["title", "issue"],
        optional: &[
            "publisher",
            "year",
            "volume",
            "variant",
            "printing",
            "grader",
            "grade",
            "cert_number",
            "signed",
            "key_issue",
        ],
    },
    CollectibleType {
        id: "trading_card",
        label: "Trading card",
        required: &["player_or_character", "set"],
        optional: &[
            "year",
            "card_number",
            "parallel",
            "serial_number",
            "print_run",
            "grader",
            "grade",
            "cert_number",
            "autographed",
            "rookie",
        ],
    },
    CollectibleType {
        id: "tcg_card",
        label: "TCG card (Magic, Pokémon, Yu-Gi-Oh)",
        required: &["name", "set"],
        optional: &[
            "game",
            "set_code",
            "collector_number",
            "rarity",
            "finish",
            "language",
            "edition",
            "grader",
            "grade",
            "cert_number",
        ],
    },
    CollectibleType {
        id: "numismatic_coin",
        label: "Coin (numismatic)",
        required: &["denomination", "year"],
        optional: &[
            "mintmark",
            "variety",
            "grader",
            "grade",
            "cert_number",
            "designation",
            "country",
        ],
    },
];

pub fn collectible_type(id: &str) -> Option<&'static CollectibleType> {
    COLLECTIBLE_TYPES.iter().find(|t| t.id == id)
}

/// Cap on any single attribute, so a pasted document cannot bloat a row.
const MAX_FIELD_LEN: usize = 2_000;

/// Validated attributes for one collectible.
pub type Attributes = BTreeMap<String, String>;

/// Validate attributes against a type's schema.
///
/// Returns the cleaned map: trimmed values, empty optionals dropped, and
/// unknown keys **preserved**. Dropping unknown keys would silently discard
/// data written by a newer version of the app.
pub fn validate(type_id: &str, attrs: &Attributes) -> Result<Attributes, CollectibleError> {
    let schema = collectible_type(type_id)
        .ok_or_else(|| CollectibleError::UnknownType(type_id.to_string()))?;

    let mut cleaned = Attributes::new();

    for (key, value) in attrs {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue; // an empty optional is simply absent
        }
        if trimmed.chars().count() > MAX_FIELD_LEN {
            return Err(CollectibleError::TooLong { field: key.clone(), max: MAX_FIELD_LEN });
        }
        cleaned.insert(key.clone(), trimmed.to_string());
    }

    for field in schema.required {
        if !cleaned.contains_key(*field) {
            return Err(CollectibleError::MissingField {
                type_id: type_id.to_string(),
                field: (*field).to_string(),
            });
        }
    }

    validate_grade(&cleaned)?;
    validate_numeric(&cleaned, &["year", "print_run"])?;

    Ok(cleaned)
}

/// A grade must be valid on its grader's own scale.
fn validate_grade(attrs: &Attributes) -> Result<(), CollectibleError> {
    let Some(raw_grade) = attrs.get("grade") else { return Ok(()) };

    // A grade without a grader is meaningless: 9.8 from whom?
    let grader_name = attrs.get("grader").map(String::as_str).unwrap_or("raw");
    let grader = Grader::parse(grader_name)
        .ok_or_else(|| CollectibleError::UnknownGrader(grader_name.to_string()))?;

    let grade: Decimal = raw_grade.parse().map_err(|_| CollectibleError::NotANumber {
        field: "grade".into(),
        value: raw_grade.clone(),
    })?;

    if !grader.accepts(grade) {
        let (min, max, step) = grader.scale();
        return Err(CollectibleError::GradeOutOfScale {
            grader: grader.display_name().to_string(),
            value: raw_grade.clone(),
            min: min.normalize().to_string(),
            max: max.normalize().to_string(),
            step: step.normalize().to_string(),
        });
    }
    Ok(())
}

fn validate_numeric(attrs: &Attributes, fields: &[&str]) -> Result<(), CollectibleError> {
    for field in fields {
        let Some(value) = attrs.get(*field) else { continue };
        if value.parse::<i64>().is_err() {
            return Err(CollectibleError::NotANumber {
                field: (*field).to_string(),
                value: value.clone(),
            });
        }
    }
    Ok(())
}

/// A short human label for a collectible, built from its attributes.
pub fn describe(type_id: &str, attrs: &Attributes) -> String {
    let get = |key: &str| attrs.get(key).map(String::as_str).unwrap_or("");

    let base = match type_id {
        "comic" => {
            let issue = get("issue");
            format!("{} #{}", get("title"), issue).trim_end_matches(" #").to_string()
        }
        "trading_card" => {
            format!("{} {} {}", get("year"), get("set"), get("player_or_character"))
        }
        "tcg_card" => format!("{} ({})", get("name"), get("set")),
        "numismatic_coin" => {
            format!("{} {} {}", get("year"), get("denomination"), get("mintmark"))
        }
        _ => get("name").to_string(),
    };

    let label = base.split_whitespace().collect::<Vec<_>>().join(" ");

    // Grade is the most price-relevant fact, so it belongs in the label.
    match (attrs.get("grader"), attrs.get("grade")) {
        (Some(grader), Some(grade)) => {
            let name = Grader::parse(grader)
                .map(|g| g.display_name().to_string())
                .unwrap_or_else(|| grader.clone());
            format!("{label} — {name} {grade}")
        }
        _ => label,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn attrs(pairs: &[(&str, &str)]) -> Attributes {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn graders_round_trip_and_accept_aliases() {
        for grader in Grader::ALL {
            assert_eq!(Grader::parse(grader.as_str()), Some(*grader));
        }
        assert_eq!(Grader::parse("Beckett"), Some(Grader::Bgs));
        assert_eq!(Grader::parse("PSA"), Some(Grader::Psa));
        assert_eq!(Grader::parse("ungraded"), Some(Grader::Raw));
        assert_eq!(Grader::parse("madeup"), None);
    }

    #[test]
    fn card_and_comic_grades_run_to_ten_in_tenths() {
        assert!(Grader::Psa.accepts(d("10")));
        assert!(Grader::Psa.accepts(d("9.5")));
        assert!(Grader::Cgc.accepts(d("9.8")));
        assert!(Grader::Cgc.accepts(d("0.5")));

        assert!(!Grader::Psa.accepts(d("11")), "above the scale");
        assert!(!Grader::Psa.accepts(d("0")), "below the scale");
        assert!(!Grader::Psa.accepts(d("9.75")), "not on a tenth boundary");
    }

    #[test]
    fn beckett_uses_half_point_steps() {
        assert!(Grader::Bgs.accepts(d("9.5")));
        assert!(Grader::Bgs.accepts(d("10")));
        assert!(!Grader::Bgs.accepts(d("9.8")), "BGS does not grade in tenths");
    }

    #[test]
    fn coin_graders_use_the_sheldon_scale() {
        // The distinction that matters: PSA 65 does not exist, PCGS 65 does.
        assert!(Grader::Pcgs.accepts(d("65")));
        assert!(Grader::Ngc.accepts(d("70")));
        assert!(Grader::Pcgs.accepts(d("1")));

        assert!(!Grader::Pcgs.accepts(d("9.8")), "Sheldon grades are whole numbers");
        assert!(!Grader::Pcgs.accepts(d("71")));
        assert!(!Grader::Psa.accepts(d("65")), "a card grader has no 65");
    }

    #[test]
    fn an_ungraded_item_has_no_numeric_grade() {
        assert!(!Grader::Raw.accepts(d("9.8")));
        assert!(!Grader::Raw.accepts(Decimal::ZERO));
    }

    #[test]
    fn required_fields_are_enforced() {
        let missing = attrs(&[("title", "Amazing Fantasy")]);
        assert!(matches!(
            validate("comic", &missing),
            Err(CollectibleError::MissingField { .. })
        ));

        let complete = attrs(&[("title", "Amazing Fantasy"), ("issue", "15")]);
        assert!(validate("comic", &complete).is_ok());
    }

    #[test]
    fn a_grade_is_checked_against_its_own_graders_scale() {
        let bad = attrs(&[
            ("title", "Amazing Fantasy"),
            ("issue", "15"),
            ("grader", "psa"),
            ("grade", "65"),
        ]);
        let err = validate("comic", &bad).unwrap_err();
        assert!(matches!(err, CollectibleError::GradeOutOfScale { .. }));
        // The message must say what a valid grade looks like.
        assert!(err.to_string().contains("0.5"));

        let good = attrs(&[
            ("title", "Amazing Fantasy"),
            ("issue", "15"),
            ("grader", "cgc"),
            ("grade", "9.8"),
        ]);
        assert!(validate("comic", &good).is_ok());
    }

    #[test]
    fn a_grade_without_a_grader_is_treated_as_ungraded_and_refused() {
        // "9.8" from whom? An unattributed grade is not a fact.
        let orphan = attrs(&[("title", "X"), ("issue", "1"), ("grade", "9.8")]);
        assert!(validate("comic", &orphan).is_err());
    }

    #[test]
    fn unknown_keys_are_preserved_not_dropped() {
        // A newer version of the app may write fields this one does not know;
        // discarding them would lose the user's data on a round trip.
        let extra = attrs(&[
            ("title", "Amazing Fantasy"),
            ("issue", "15"),
            ("some_future_field", "keep me"),
        ]);
        let cleaned = validate("comic", &extra).unwrap();
        assert_eq!(cleaned.get("some_future_field").map(String::as_str), Some("keep me"));
    }

    #[test]
    fn values_are_trimmed_and_empty_optionals_dropped() {
        let messy =
            attrs(&[("title", "  Amazing Fantasy  "), ("issue", "15"), ("variant", "   ")]);
        let cleaned = validate("comic", &messy).unwrap();
        assert_eq!(cleaned.get("title").map(String::as_str), Some("Amazing Fantasy"));
        assert!(!cleaned.contains_key("variant"), "a blank optional is absent, not empty");
    }

    #[test]
    fn an_empty_required_field_is_missing_not_blank() {
        let blank = attrs(&[("title", "X"), ("issue", "   ")]);
        assert!(matches!(
            validate("comic", &blank),
            Err(CollectibleError::MissingField { .. })
        ));
    }

    #[test]
    fn oversized_values_are_refused() {
        let huge = attrs(&[
            ("title", "X"),
            ("issue", "1"),
            ("notes_field", &"x".repeat(MAX_FIELD_LEN + 1)),
        ]);
        assert!(matches!(validate("comic", &huge), Err(CollectibleError::TooLong { .. })));
    }

    #[test]
    fn numeric_fields_are_checked() {
        let bad_year = attrs(&[("title", "X"), ("issue", "1"), ("year", "nineteen sixty-two")]);
        assert!(matches!(
            validate("comic", &bad_year),
            Err(CollectibleError::NotANumber { .. })
        ));
    }

    #[test]
    fn leading_zeros_survive_in_identifiers() {
        // Cert and card numbers are identifiers, not quantities; "0012345"
        // must not become 12345.
        let card = attrs(&[
            ("player_or_character", "Michael Jordan"),
            ("set", "Fleer"),
            ("cert_number", "0012345"),
            ("card_number", "057"),
        ]);
        let cleaned = validate("trading_card", &card).unwrap();
        assert_eq!(cleaned.get("cert_number").map(String::as_str), Some("0012345"));
        assert_eq!(cleaned.get("card_number").map(String::as_str), Some("057"));
    }

    #[test]
    fn unknown_types_are_refused() {
        assert!(matches!(
            validate("stamp_collection", &attrs(&[])),
            Err(CollectibleError::UnknownType(_))
        ));
    }

    #[test]
    fn labels_include_the_grade_because_it_drives_the_price() {
        let comic = attrs(&[
            ("title", "Amazing Fantasy"),
            ("issue", "15"),
            ("grader", "cgc"),
            ("grade", "9.8"),
        ]);
        assert_eq!(describe("comic", &comic), "Amazing Fantasy #15 — CGC 9.8");

        let ungraded = attrs(&[("title", "Amazing Fantasy"), ("issue", "15")]);
        assert_eq!(describe("comic", &ungraded), "Amazing Fantasy #15");
    }

    #[test]
    fn labels_handle_each_type() {
        assert_eq!(
            describe("tcg_card", &attrs(&[("name", "Black Lotus"), ("set", "Alpha")])),
            "Black Lotus (Alpha)"
        );
        assert_eq!(
            describe(
                "numismatic_coin",
                &attrs(&[("year", "1909"), ("denomination", "Cent"), ("mintmark", "S")])
            ),
            "1909 Cent S"
        );
    }

    #[test]
    fn every_bundled_type_validates_its_own_required_fields() {
        // Numeric fields need a number even when they are required, so the
        // filler has to respect that — numismatic_coin requires `year`.
        let filler = |field: &str| {
            if ["year", "print_run"].contains(&field) {
                "1962"
            } else {
                "value"
            }
        };

        for collectible in COLLECTIBLE_TYPES {
            let filled: Attributes = collectible
                .required
                .iter()
                .map(|f| ((*f).to_string(), filler(f).to_string()))
                .collect();
            assert!(
                validate(collectible.id, &filled).is_ok(),
                "type {} cannot satisfy its own required fields",
                collectible.id
            );
        }
    }

    #[test]
    fn a_required_numeric_field_still_has_to_be_numeric() {
        // The case the test above tripped over: `year` is both required and
        // numeric on a coin, so filling it with text must fail.
        let bad = attrs(&[("denomination", "Cent"), ("year", "nineteen-oh-nine")]);
        assert!(matches!(
            validate("numismatic_coin", &bad),
            Err(CollectibleError::NotANumber { .. })
        ));

        let good = attrs(&[("denomination", "Cent"), ("year", "1909")]);
        assert!(validate("numismatic_coin", &good).is_ok());
    }
}
