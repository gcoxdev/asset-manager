//! Reading someone's own spreadsheet.
//!
//! The round-trip CSV (`csv`) uses the app's own columns and amounts in
//! cents, which is exact but not what anyone's household inventory looks
//! like. This reads an ordinary table — "Item, Purchase price, Date bought,
//! Where" — guesses which column is which, and normalizes the values the
//! owner says how to read: date order, decimal comma. Turning a row into an
//! asset, and validating it, is left to the same code that saves the add
//! form, so an imported asset obeys exactly the same rules as a typed one.

use serde::{Deserialize, Serialize};

use crate::csv::{parse_delimited, CsvError, MAX_FILE_BYTES};

/// A spreadsheet as read: a header row and the rows under it.
#[derive(Debug, Clone, Serialize)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// The separator that was detected: ",", ";" or "\t".
    pub delimiter: String,
}

/// Read a delimited text file. Strips a byte-order mark, detects the
/// separator from the header line, and drops blank rows.
pub fn read_table(input: &str) -> Result<Table, CsvError> {
    if input.len() > MAX_FILE_BYTES {
        return Err(CsvError::TooLarge);
    }
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let first_line = input.lines().next().unwrap_or("");
    let delimiter = [',', ';', '\t']
        .into_iter()
        .max_by_key(|d| first_line.matches(*d).count())
        .filter(|d| first_line.contains(*d))
        .unwrap_or(',');
    let mut rows = parse_delimited(input, delimiter)?;
    rows.retain(|r| r.iter().any(|c| !c.trim().is_empty()));
    if rows.is_empty() {
        return Err(CsvError::Malformed { line: 1, reason: "the file is empty".into() });
    }
    let headers: Vec<String> =
        rows.remove(0).into_iter().map(|h| h.trim().to_string()).collect();
    Ok(Table { headers, rows, delimiter: delimiter.to_string() })
}

/// Where a column's values go. Serialized as plain strings for the frontend:
/// `"name"`, `"acquired_price"`, `"detail:serial_number"`, `"ignore"`.
pub const FIELDS: &[&str] = &[
    "name",
    "type",
    "quantity",
    "unit",
    "acquired_date",
    "acquired_price",
    "acquired_from",
    "storage_location",
    "notes",
    "current_value",
    "insured_value",
    "currency",
    "tags",
];

fn words(header: &str) -> String {
    header
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A detail key from a header: "Serial #" → "serial_number", "Box size" →
/// "box_size".
pub fn detail_key(header: &str) -> String {
    let w = words(header);
    match w.as_str() {
        "serial" | "serial number" | "serial no" | "sn" | "s n" | "serial num" => {
            "serial_number".into()
        }
        "cert" | "certificate" | "cert number" | "cert no" | "certificate number" => {
            "cert_number".into()
        }
        "make" | "brand" => "brand".into(),
        _ => w.replace(' ', "_"),
    }
}

/// Guess what each column holds from its header. Anything unrecognized is
/// kept as a detail under its own name, so nothing in the file is silently
/// dropped; the owner can set any column to "ignore".
pub fn guess_mapping(headers: &[String]) -> Vec<String> {
    let mut used: Vec<&str> = Vec::new();
    headers
        .iter()
        .map(|header| {
            let w = words(header);
            let field = match w.as_str() {
                "name" | "item" | "item name" | "description" | "title" | "asset" | "what" => {
                    "name"
                }
                "type" | "category" | "kind" | "asset type" | "item type" => "type",
                "qty" | "quantity" | "count" | "number owned" | "how many" => "quantity",
                "unit" | "units" => "unit",
                "date" | "purchase date" | "date purchased" | "acquired" | "date acquired"
                | "bought" | "date bought" | "purchased" | "acquisition date" => {
                    "acquired_date"
                }
                "price" | "cost" | "paid" | "purchase price" | "price paid" | "cost basis"
                | "total paid" | "amount paid" | "purchase cost" => "acquired_price",
                "seller" | "vendor" | "store" | "dealer" | "purchased from" | "bought from"
                | "source" | "from" => "acquired_from",
                "location" | "where" | "stored" | "storage" | "storage location" | "room"
                | "kept" | "where kept" => "storage_location",
                "notes" | "note" | "comments" | "comment" | "remarks" => "notes",
                "value" | "current value" | "estimated value" | "market value" | "worth"
                | "appraised value" | "est value" => "current_value",
                "insured" | "insured value" | "insurance" | "insurance value" | "coverage"
                | "insured for" => "insured_value",
                "currency" | "ccy" => "currency",
                "tags" | "tag" | "labels" => "tags",
                _ => "",
            };
            // Each field takes the first column that claims it; a second
            // "Notes" column is kept as a detail rather than overwriting.
            if !field.is_empty() && !used.contains(&field) {
                used.push(field);
                field.to_string()
            } else if w.is_empty() {
                "ignore".to_string()
            } else {
                format!("detail:{}", detail_key(header))
            }
        })
        .collect()
}

/// How dates in the file are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DateOrder {
    /// 2024-03-15
    YearMonthDay,
    /// 03/15/2024
    MonthDayYear,
    /// 15/03/2024
    DayMonthYear,
}

/// A date as written in the file → `YYYY-MM-DD`. Separators may be `-`,
/// `/` or `.`; a two-digit year is read as 19xx from 50 up, 20xx below.
pub fn normalize_date(raw: &str, order: DateOrder) -> Result<String, String> {
    let raw = raw.trim();
    let parts: Vec<&str> = raw.split(['-', '/', '.']).map(str::trim).collect();
    let bad = || format!("{raw:?} is not a date in the chosen format");
    if parts.len() != 3
        || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        // Already ISO with a time part ("2024-03-15T10:00:00") is fine too.
        if raw.len() > 10 && raw.as_bytes().get(10) == Some(&b'T') {
            return crate::events::normalize_date(raw).map_err(|_| bad());
        }
        return Err(bad());
    }
    // An ISO date is unambiguous whatever order was chosen.
    let (y, m, d) = if parts[0].len() == 4 {
        (parts[0], parts[1], parts[2])
    } else {
        match order {
            DateOrder::YearMonthDay => (parts[0], parts[1], parts[2]),
            DateOrder::MonthDayYear => (parts[2], parts[0], parts[1]),
            DateOrder::DayMonthYear => (parts[2], parts[1], parts[0]),
        }
    };
    let year: i64 = y.parse().map_err(|_| bad())?;
    let year = match y.len() {
        2 if year >= 50 => 1900 + year,
        2 => 2000 + year,
        4 => year,
        _ => return Err(bad()),
    };
    let month: u32 = m.parse().map_err(|_| bad())?;
    let day: u32 = d.parse().map_err(|_| bad())?;
    if !crate::series::is_calendar_date(year, month, day) {
        return Err(bad());
    }
    Ok(format!("{year:04}-{month:02}-{day:02}"))
}

/// A number as written in the file → one the app parses. With a decimal
/// comma, "1.234,56" is 1234.56; without, "1,234.56" is. Currency symbols
/// are left for the money parser, which tolerates them.
pub fn normalize_number(raw: &str, decimal_comma: bool) -> String {
    let raw = raw.trim();
    if decimal_comma {
        raw.chars()
            .filter(|c| *c != '.' && *c != ' ' && *c != '\u{a0}')
            .map(|c| if c == ',' { '.' } else { c })
            .collect()
    } else {
        raw.chars().filter(|c| *c != ' ' && *c != '\u{a0}').collect()
    }
}

/// Common words for types, beyond their own names.
const TYPE_SYNONYMS: &[(&str, &str)] = &[
    ("car", "vehicle"),
    ("truck", "vehicle"),
    ("motorcycle", "vehicle"),
    ("motorbike", "vehicle"),
    ("van", "vehicle"),
    ("rv", "vehicle"),
    ("trailer", "vehicle"),
    ("house", "real_estate"),
    ("home", "real_estate"),
    ("apartment", "real_estate"),
    ("condo", "real_estate"),
    ("property", "real_estate"),
    ("laptop", "electronics"),
    ("computer", "electronics"),
    ("phone", "electronics"),
    ("tv", "electronics"),
    ("television", "electronics"),
    ("camera", "electronics"),
    ("tablet", "electronics"),
    ("fridge", "appliance"),
    ("washer", "appliance"),
    ("dryer", "appliance"),
    ("sofa", "furniture"),
    ("couch", "furniture"),
    ("table", "furniture"),
    ("chair", "furniture"),
    ("bed", "furniture"),
    ("antique", "furniture"),
    ("tool", "tools"),
    ("equipment", "tools"),
    ("bike", "sports_gear"),
    ("bicycle", "sports_gear"),
    ("golf clubs", "sports_gear"),
    ("handbag", "fashion"),
    ("bag", "fashion"),
    ("purse", "fashion"),
    ("shoes", "fashion"),
    ("sneakers", "fashion"),
    ("book", "books"),
    ("stock", "security"),
    ("shares", "security"),
    ("bond", "security"),
    ("fund", "security"),
    ("etf", "security"),
    ("lego", "toy"),
    ("action figure", "toy"),
    ("gun", "firearm"),
    ("rifle", "firearm"),
    ("pistol", "firearm"),
    ("handgun", "firearm"),
    ("shotgun", "firearm"),
    ("ammo", "ammunition"),
    ("scope", "firearm_accessory"),
    ("optic", "firearm_accessory"),
    ("comic", "comic"),
    ("comic book", "comic"),
    ("sports card", "trading_card"),
    ("baseball card", "trading_card"),
    ("card", "trading_card"),
    ("pokemon", "tcg_card"),
    ("magic", "tcg_card"),
    ("coin", "numismatic_coin"),
    ("collector coin", "numismatic_coin"),
    ("gold", "gold_bullion"),
    ("silver", "silver_bullion"),
    ("platinum", "platinum_bullion"),
    ("palladium", "palladium_bullion"),
    ("watches", "watch"),
    ("jewellery", "jewelry"),
    ("ring", "jewelry"),
    ("necklace", "jewelry"),
    ("painting", "art"),
    ("print", "art"),
    ("artwork", "art"),
    ("guitar", "instrument"),
    ("musical instrument", "instrument"),
    ("wine", "wine"),
    ("whisky", "wine"),
    ("whiskey", "wine"),
    ("bourbon", "wine"),
    ("record", "vinyl"),
    ("vinyl record", "vinyl"),
    ("lp", "vinyl"),
    ("game", "video_game"),
    ("cash", "cash"),
    ("bitcoin", "crypto"),
    ("crypto", "crypto"),
];

/// Match a type written in the file against the app's types: by ID, by
/// label, or by a common word for it. `types` is (type_id, label).
pub fn match_type(value: &str, types: &[(String, String)]) -> Option<String> {
    let w = words(value);
    if w.is_empty() {
        return None;
    }
    let singular = w.strip_suffix('s').unwrap_or(&w).to_string();
    for candidate in [&w, &singular] {
        if let Some((id, _)) = types.iter().find(|(id, label)| {
            words(id) == *candidate
                || words(&id.replace('_', " ")) == *candidate
                || words(label) == *candidate
        }) {
            return Some(id.clone());
        }
        if let Some((_, id)) =
            TYPE_SYNONYMS.iter().find(|(word, _)| *word == candidate.as_str())
        {
            if types.iter().any(|(t, _)| t == id) {
                return Some((*id).to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_household_spreadsheet_is_read_and_its_columns_guessed() {
        let file = "\u{feff}Item;Purchase Price;Date bought;Where;Serial #;Notes;Comments\n\
                    Rolex Submariner;8.500,00;15/03/2021;Safe;7A1234;Gift from Dad;\n\
                    ;;;;;;\n";
        let table = read_table(file).unwrap();
        assert_eq!(table.delimiter, ";");
        assert_eq!(table.rows.len(), 1, "blank rows dropped");
        assert_eq!(
            guess_mapping(&table.headers),
            vec![
                "name",
                "acquired_price",
                "acquired_date",
                "storage_location",
                "detail:serial_number",
                "notes",
                "detail:comments",
            ]
        );
    }

    #[test]
    fn dates_are_read_in_the_order_the_owner_says() {
        assert_eq!(
            normalize_date("03/04/2024", DateOrder::MonthDayYear).unwrap(),
            "2024-03-04"
        );
        assert_eq!(
            normalize_date("03/04/2024", DateOrder::DayMonthYear).unwrap(),
            "2024-04-03"
        );
        assert_eq!(
            normalize_date("2024-04-03", DateOrder::DayMonthYear).unwrap(),
            "2024-04-03",
            "ISO is unambiguous"
        );
        assert_eq!(normalize_date("1.2.99", DateOrder::DayMonthYear).unwrap(), "1999-02-01");
        assert_eq!(normalize_date("1/2/07", DateOrder::MonthDayYear).unwrap(), "2007-01-02");
        assert!(
            normalize_date("31/02/2024", DateOrder::DayMonthYear).is_err(),
            "no 31 February"
        );
        assert!(normalize_date("last spring", DateOrder::YearMonthDay).is_err());
    }

    #[test]
    fn numbers_are_read_with_the_owners_decimal_mark() {
        assert_eq!(normalize_number("8.500,00", true), "8500.00");
        assert_eq!(
            normalize_number("$8,500.00", false),
            "$8,500.00",
            "left for the money parser"
        );
        assert_eq!(normalize_number(" 1 234,5 ", true), "1234.5");
    }

    #[test]
    fn types_are_matched_by_id_label_or_common_word() {
        let types = vec![
            ("watch".to_string(), "Watch".to_string()),
            ("trading_card".to_string(), "Trading Card".to_string()),
            ("firearm".to_string(), "Firearm".to_string()),
        ];
        assert_eq!(match_type("Watches", &types).as_deref(), Some("watch"));
        assert_eq!(match_type("trading card", &types).as_deref(), Some("trading_card"));
        assert_eq!(match_type("Rifle", &types).as_deref(), Some("firearm"));
        assert_eq!(match_type("Spaceship", &types), None);
    }

    #[test]
    fn a_second_column_for_the_same_field_is_kept_as_a_detail() {
        let headers = vec!["Notes".to_string(), "Note".to_string(), "".to_string()];
        assert_eq!(guess_mapping(&headers), vec!["notes", "detail:note", "ignore"]);
    }
}
