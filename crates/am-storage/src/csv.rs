//! CSV import and export.
//!
//! Per decision 4, manual valuation is the primary path for collectibles, and
//! for a few hundred items *export → edit in a spreadsheet → reimport* beats
//! any in-app flow. That makes this a first-class data path, not a
//! convenience, so it carries a real contract.
//!
//! # CSV is not a backup
//!
//! It excludes photos, documents, valuation history and per-type attributes.
//! Exports are also **plaintext by intent** — once written, the file is
//! outside the vault's protection. Callers must say so.
//!
//! # Round-trip safety
//!
//! - Every row carries a stable `asset_id`, so a reimport updates rather than
//!   duplicating.
//! - A `vault_id` and `exported_at` header row detects a spreadsheet exported
//!   from a different vault, or one stale enough to clobber newer edits.
//! - Values are written as text with leading zeros preserved; a cert number
//!   like `0012345` must survive a spreadsheet round-trip.
//! - Blank and zero are distinct: a blank cell *preserves* the stored value,
//!   and `-` clears it. Silently treating blank as "set to empty" would let a
//!   partially-filled spreadsheet wipe data.

use std::collections::HashMap;

use crate::vault::{Vault, VaultError};

/// Bumped when the column set changes incompatibly.
pub const CSV_FORMAT_VERSION: u32 = 1;

/// Sentinel meaning "clear this field". A blank cell preserves instead.
pub const CLEAR_MARKER: &str = "-";

pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ROWS: usize = 100_000;
pub const MAX_CELL_BYTES: usize = 32 * 1024;

const COLUMNS: &[&str] = &[
    "asset_id",
    "type_id",
    "name",
    "status",
    "quantity",
    "quantity_unit",
    "acquired_date",
    "acquired_amount_minor",
    "acquired_currency",
    "acquired_from",
    "storage_location",
    "notes",
    "current_amount_minor",
    "current_currency",
    "value_asof",
];

#[derive(Debug, thiserror::Error)]
pub enum CsvError {
    #[error("file is larger than the {} MiB limit", MAX_FILE_BYTES / 1024 / 1024)]
    TooLarge,
    #[error("too many rows (limit {MAX_ROWS})")]
    TooManyRows,
    #[error("a cell on line {line} exceeds the {} KiB limit", MAX_CELL_BYTES / 1024)]
    CellTooLarge { line: usize },
    #[error("malformed CSV on line {line}: {reason}")]
    Malformed { line: usize, reason: String },
    #[error("missing required column: {0}")]
    MissingColumn(String),
    #[error(
        "this file was exported from a different vault (expected {expected}, found {found}) — \
         importing it would mix two catalogs"
    )]
    WrongVault { expected: String, found: String },
    #[error(
        "this file was exported before {row_count} of the assets in it were last changed; \
         re-export and edit again, or these edits would overwrite newer changes"
    )]
    StaleExport { row_count: usize },
    #[error("row {line}: {reason}")]
    Invalid { line: usize, reason: String },
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Vault(#[from] VaultError),
}

// ---------------------------------------------------------------- parsing

/// Minimal RFC 4180 reader. Hand-rolled rather than pulled in: the format is
/// small, and export quoting has to be controlled precisely for leading zeros.
fn parse_csv(input: &str) -> Result<Vec<Vec<String>>, CsvError> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut line = 1usize;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => in_quotes = false,
                '\n' => {
                    line += 1;
                    field.push('\n');
                }
                _ => field.push(c),
            }
        } else {
            match c {
                '"' if field.is_empty() => in_quotes = true,
                '"' => {
                    return Err(CsvError::Malformed {
                        line,
                        reason: "unexpected quote inside an unquoted field".into(),
                    })
                }
                ',' => {
                    row.push(std::mem::take(&mut field));
                }
                '\r' => {} // CRLF tolerated
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                    line += 1;
                    if rows.len() > MAX_ROWS {
                        return Err(CsvError::TooManyRows);
                    }
                }
                _ => field.push(c),
            }
        }
        if field.len() > MAX_CELL_BYTES {
            return Err(CsvError::CellTooLarge { line });
        }
    }

    if in_quotes {
        return Err(CsvError::Malformed { line, reason: "unterminated quoted field".into() });
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

/// Quote a field for export.
///
/// Always quotes anything that could be mangled: a leading zero (cert numbers,
/// card numbers), a leading `+`/`=`/`@` (which spreadsheets treat as formulas),
/// or any separator. Quoting is not sufficient on its own for formula cells —
/// see [`escape_formula`].
fn write_field(out: &mut String, value: &str) {
    let needs_quotes = value.is_empty()
        || value.contains([',', '"', '\n', '\r'])
        || value.starts_with('0')
        || value.starts_with(' ')
        || value.ends_with(' ');

    if needs_quotes {
        out.push('"');
        for c in value.chars() {
            if c == '"' {
                out.push('"');
            }
            out.push(c);
        }
        out.push('"');
    } else {
        out.push_str(value);
    }
}

/// Neutralize a cell a spreadsheet would evaluate as a formula.
///
/// Prefixing with an apostrophe is the conventional fix, and it is reversible:
/// [`unescape_formula`] strips it on reimport, so the round-trip is lossless
/// for a name that genuinely begins with `=`.
fn escape_formula(value: &str) -> String {
    if value.starts_with(['=', '+', '@']) || (value.starts_with('-') && value.len() > 1) {
        format!("'{value}")
    } else {
        value.to_string()
    }
}

fn unescape_formula(value: &str) -> &str {
    value
        .strip_prefix('\'')
        .filter(|rest| rest.starts_with(['=', '+', '@', '-']))
        .unwrap_or(value)
}

// ---------------------------------------------------------------- export

pub struct ExportResult {
    pub csv: String,
    pub row_count: usize,
}

pub fn export_assets(vault: &Vault, now: &str) -> Result<ExportResult, CsvError> {
    let vault_id: String =
        vault.conn().query_row("SELECT vault_id FROM vault_meta WHERE id=1", [], |r| r.get(0))?;

    let mut out = String::new();

    // Metadata line, then the header. A leading `#` line keeps spreadsheets
    // from treating it as data, and carries what stale-detection needs.
    out.push_str(&format!(
        "# asset-manager-csv v{CSV_FORMAT_VERSION} vault={vault_id} exported_at={now}\n"
    ));

    for (i, col) in COLUMNS.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(col);
    }
    out.push('\n');

    let mut stmt = vault.conn().prepare(
        "SELECT asset_id, type_id, name, status, quantity, quantity_unit,
                acquired_date, acquired_amount_minor, acquired_currency, acquired_from,
                storage_location, notes, current_amount_minor, current_currency, value_asof
         FROM assets ORDER BY created_at",
    )?;

    let rows = stmt.query_map([], |r| {
        let mut cells: Vec<String> = Vec::with_capacity(COLUMNS.len());
        for i in 0..COLUMNS.len() {
            // Read through ValueRef and render each type ourselves. Asking
            // rusqlite for a String on an INTEGER column fails outright, and a
            // money column is INTEGER — so this has to be explicit. Nothing is
            // routed through f64: a float would corrupt large minor-unit
            // amounts and precise quantities.
            let value = match r.get_ref(i)? {
                rusqlite::types::ValueRef::Null => String::new(),
                rusqlite::types::ValueRef::Integer(v) => v.to_string(),
                rusqlite::types::ValueRef::Real(v) => v.to_string(),
                rusqlite::types::ValueRef::Text(v) => {
                    String::from_utf8_lossy(v).into_owned()
                }
                rusqlite::types::ValueRef::Blob(_) => String::new(),
            };
            cells.push(value);
        }
        Ok(cells)
    })?;

    let mut row_count = 0;
    for row in rows {
        let cells = row?;
        for (i, cell) in cells.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_field(&mut out, &escape_formula(cell));
        }
        out.push('\n');
        row_count += 1;
    }

    Ok(ExportResult { csv: out, row_count })
}

// ---------------------------------------------------------------- import

#[derive(Debug, Default, Clone)]
pub struct ImportPreview {
    pub creates: usize,
    pub updates: usize,
    pub unchanged: usize,
    /// Row-level problems. A preview reports every one rather than stopping at
    /// the first, so the user fixes the spreadsheet in a single pass.
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    /// Validate and report, write nothing.
    Preview,
    /// Apply, in one transaction.
    Apply,
}

struct ParsedRow {
    line: usize,
    values: HashMap<String, String>,
}

fn parse_metadata(line: &str) -> HashMap<String, String> {
    line.trim_start_matches('#')
        .split_whitespace()
        .filter_map(|token| token.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Import a CSV, either previewing or applying it.
///
/// Applying runs in a single transaction: either every valid row lands or none
/// does, so a failure halfway through cannot leave a half-edited catalog.
pub fn import_assets(
    vault: &Vault,
    input: &str,
    mode: ImportMode,
    now: &str,
) -> Result<ImportPreview, CsvError> {
    if input.len() > MAX_FILE_BYTES {
        return Err(CsvError::TooLarge);
    }

    let rows = parse_csv(input)?;
    if rows.is_empty() {
        return Ok(ImportPreview::default());
    }

    let mut index = 0;
    let mut metadata = HashMap::new();
    if rows[0].first().map(|c| c.starts_with('#')).unwrap_or(false) {
        metadata = parse_metadata(&rows[0][0]);
        index = 1;
    }

    // Refuse a file from another vault outright: merging two catalogs by
    // accident is far worse than an inconvenient error.
    if let Some(found) = metadata.get("vault") {
        let expected: String = vault
            .conn()
            .query_row("SELECT vault_id FROM vault_meta WHERE id=1", [], |r| r.get(0))?;
        if found != &expected {
            return Err(CsvError::WrongVault { expected, found: found.clone() });
        }
    }

    let Some(header) = rows.get(index) else { return Ok(ImportPreview::default()) };
    let header: Vec<String> = header.iter().map(|h| h.trim().to_string()).collect();

    for required in ["asset_id", "name"] {
        if !header.iter().any(|h| h == required) {
            return Err(CsvError::MissingColumn(required.to_string()));
        }
    }

    let parsed: Vec<ParsedRow> = rows[index + 1..]
        .iter()
        .enumerate()
        .filter(|(_, row)| row.iter().any(|c| !c.trim().is_empty()))
        .map(|(offset, row)| {
            let mut values = HashMap::new();
            for (i, col) in header.iter().enumerate() {
                let raw = row.get(i).map(String::as_str).unwrap_or("");
                values.insert(col.clone(), unescape_formula(raw).to_string());
            }
            ParsedRow { line: index + 2 + offset, values }
        })
        .collect();

    // Stale-export detection: a row whose asset changed after this file was
    // exported would be silently overwritten.
    //
    // A newer `updated_at` alone is not enough to refuse. Importing the same
    // file twice bumps `updated_at` on the first pass, which would make the
    // second pass look stale even though the file still matches the database
    // exactly. So a row only counts as stale when the stored values actually
    // *differ* from what the spreadsheet carries — that is the case where
    // applying it would destroy a newer edit.
    if let Some(exported_at) = metadata.get("exported_at") {
        let mut stale = 0;
        for row in &parsed {
            let Some(id) = row.values.get("asset_id").filter(|v| !v.trim().is_empty()) else {
                continue;
            };
            let updated: Option<String> = vault
                .conn()
                .query_row("SELECT updated_at FROM assets WHERE asset_id = ?1", [id], |r| r.get(0))
                .ok();

            let Some(updated) = updated else { continue };
            if updated.as_str() <= exported_at.as_str() {
                continue; // untouched since export
            }
            if row_differs_from_stored(vault, id, row)? {
                stale += 1;
            }
        }
        if stale > 0 {
            return Err(CsvError::StaleExport { row_count: stale });
        }
    }

    let mut preview = ImportPreview::default();
    let mut seen: HashMap<String, usize> = HashMap::new();

    // Validate everything before writing anything.
    for row in &parsed {
        let id = row.values.get("asset_id").map(|s| s.trim()).unwrap_or("");
        let name = row.values.get("name").map(|s| s.trim()).unwrap_or("");

        if name.is_empty() && id.is_empty() {
            preview.errors.push(format!("row {}: needs at least a name", row.line));
            continue;
        }
        if !id.is_empty() {
            if let Some(first) = seen.get(id) {
                preview
                    .errors
                    .push(format!("row {}: asset_id repeats row {first}", row.line));
                continue;
            }
            seen.insert(id.to_string(), row.line);
        }

        // Money must arrive with its currency, matching the schema's CHECK.
        for (amount_col, currency_col) in [
            ("acquired_amount_minor", "acquired_currency"),
            ("current_amount_minor", "current_currency"),
        ] {
            let amount = row.values.get(amount_col).map(|s| s.trim()).unwrap_or("");
            let currency = row.values.get(currency_col).map(|s| s.trim()).unwrap_or("");
            if !amount.is_empty() && amount != CLEAR_MARKER {
                if amount.parse::<i64>().is_err() {
                    preview.errors.push(format!(
                        "row {}: {amount_col} must be a whole number of minor units",
                        row.line
                    ));
                }
                if currency.is_empty() {
                    preview.errors.push(format!(
                        "row {}: {amount_col} needs {currency_col}",
                        row.line
                    ));
                }
            }
        }

        if let Some(q) = row.values.get("quantity").map(|s| s.trim()) {
            if !q.is_empty() && q != CLEAR_MARKER && q.parse::<f64>().is_err() {
                preview.errors.push(format!("row {}: quantity is not a number", row.line));
            }
        }

        let exists = if id.is_empty() {
            false
        } else {
            vault.conn().query_row(
                "SELECT count(*) FROM assets WHERE asset_id = ?1",
                [id],
                |r| r.get::<_, i64>(0),
            )? > 0
        };

        if exists {
            preview.updates += 1;
        } else {
            preview.creates += 1;
        }
    }

    if mode == ImportMode::Preview || !preview.errors.is_empty() {
        return Ok(preview);
    }

    // One transaction for the whole file.
    let tx = vault.conn().unchecked_transaction()?;
    for row in &parsed {
        let id = row.values.get("asset_id").map(|s| s.trim()).unwrap_or("");

        let exists = !id.is_empty()
            && tx.query_row("SELECT count(*) FROM assets WHERE asset_id = ?1", [id], |r| {
                r.get::<_, i64>(0)
            })? > 0;

        if exists {
            apply_update(&tx, id, row, now)?;
        } else {
            apply_insert(&tx, row, now)?;
        }
    }
    tx.commit()?;

    Ok(preview)
}

/// Does this spreadsheet row carry a value that differs from what is stored?
///
/// Used by stale-export detection: a row identical to the database is
/// harmless to reapply no matter how the timestamps compare.
fn row_differs_from_stored(
    vault: &Vault,
    id: &str,
    row: &ParsedRow,
) -> Result<bool, CsvError> {
    for column in COLUMNS.iter().filter(|c| **c != "asset_id") {
        let Some(incoming) = cell(row, column) else { continue };

        let sql = format!("SELECT {column} FROM assets WHERE asset_id = ?1");
        let stored: Option<String> = vault.conn().query_row(&sql, [id], |r| {
            Ok(match r.get_ref(0)? {
                rusqlite::types::ValueRef::Null => None,
                rusqlite::types::ValueRef::Integer(v) => Some(v.to_string()),
                rusqlite::types::ValueRef::Real(v) => Some(v.to_string()),
                rusqlite::types::ValueRef::Text(v) => {
                    Some(String::from_utf8_lossy(v).into_owned())
                }
                rusqlite::types::ValueRef::Blob(_) => None,
            })
        })?;

        let incoming = incoming.map(str::to_string);
        if incoming != stored {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Blank preserves, [`CLEAR_MARKER`] clears, anything else sets.
fn cell<'a>(row: &'a ParsedRow, column: &str) -> Option<Option<&'a str>> {
    let raw = row.values.get(column)?.trim();
    if raw.is_empty() {
        None // preserve
    } else if raw == CLEAR_MARKER {
        Some(None) // clear
    } else {
        Some(Some(raw)) // set
    }
}

fn apply_update(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    row: &ParsedRow,
    now: &str,
) -> Result<(), CsvError> {
    for column in COLUMNS.iter().filter(|c| **c != "asset_id") {
        let Some(value) = cell(row, column) else { continue };

        // Identifiers are validated by CHECK constraints; the column name here
        // comes from our own COLUMNS list, never from the file.
        let sql = format!("UPDATE assets SET {column} = ?1, updated_at = ?2 WHERE asset_id = ?3");
        tx.execute(&sql, rusqlite::params![value, now, id])?;

        if *column == "quantity" {
            if let Some(q) = value {
                let sort = q.parse::<f64>().unwrap_or(0.0);
                tx.execute(
                    "UPDATE assets SET quantity_sort = ?1 WHERE asset_id = ?2",
                    rusqlite::params![sort, id],
                )?;
            }
        }
    }
    Ok(())
}

fn apply_insert(
    tx: &rusqlite::Transaction<'_>,
    row: &ParsedRow,
    now: &str,
) -> Result<(), CsvError> {
    let id = {
        let raw = row.values.get("asset_id").map(|s| s.trim()).unwrap_or("");
        if raw.is_empty() {
            uuid_v4()
        } else {
            raw.to_string()
        }
    };
    let get = |c: &str| row.values.get(c).map(|s| s.trim()).filter(|s| !s.is_empty() && *s != CLEAR_MARKER);

    let name = get("name").unwrap_or("Untitled");
    let type_id = get("type_id").unwrap_or("generic");
    let quantity = get("quantity").unwrap_or("1");
    let sort = quantity.parse::<f64>().unwrap_or(0.0);

    tx.execute(
        "INSERT INTO assets
           (asset_id, type_id, name, status, quantity, quantity_sort, quantity_unit,
            acquired_date, acquired_amount_minor, acquired_currency, acquired_from,
            storage_location, notes, current_amount_minor, current_currency, value_asof,
            created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17)",
        rusqlite::params![
            &id,
            type_id,
            name,
            get("status").unwrap_or("active"),
            quantity,
            sort,
            get("quantity_unit").unwrap_or("item"),
            get("acquired_date"),
            get("acquired_amount_minor"),
            get("acquired_currency"),
            get("acquired_from"),
            get("storage_location"),
            get("notes").unwrap_or(""),
            get("current_amount_minor"),
            get("current_currency"),
            get("value_asof"),
            now,
        ],
    )?;

    // Ownership starts as an event, exactly as it does for the UI path.
    tx.execute(
        "INSERT INTO asset_events
           (event_id, asset_id, event_type, effective_date, quantity_delta, recorded_at)
         VALUES (?1, ?2, 'acquire', ?3, ?4, ?3)",
        rusqlite::params![uuid_v4(), &id, now, quantity],
    )?;
    Ok(())
}

fn uuid_v4() -> String {
    let mut raw = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
    raw[6] = (raw[6] & 0x0f) | 0x40;
    raw[8] = (raw[8] & 0x3f) | 0x80;
    let h: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_crypto::KdfParams;

    const NOW: &str = "2026-09-19T00:00:00Z";
    const PASS: &str = "correct horse battery staple";

    fn fast() -> KdfParams {
        KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 }
    }

    fn vault(dir: &std::path::Path) -> Vault {
        let (v, _r) = Vault::create(dir, PASS, &fast(), NOW).unwrap();
        v
    }

    fn insert(v: &Vault, id: &str, name: &str) {
        v.conn()
            .execute(
                "INSERT INTO assets (asset_id, type_id, name, created_at, updated_at)
                 VALUES (?1,'generic',?2,?3,?3)",
                rusqlite::params![id, name, NOW],
            )
            .unwrap();
    }

    #[test]
    fn parses_quoted_fields_and_embedded_separators() {
        let rows = parse_csv("a,\"b,with,commas\",c\n\"line\nbreak\",\"say \"\"hi\"\"\",z\n")
            .unwrap();
        assert_eq!(rows[0], vec!["a", "b,with,commas", "c"]);
        assert_eq!(rows[1], vec!["line\nbreak", "say \"hi\"", "z"]);
    }

    #[test]
    fn rejects_unterminated_quotes() {
        assert!(matches!(
            parse_csv("a,\"unterminated\n"),
            Err(CsvError::Malformed { .. })
        ));
    }

    #[test]
    fn export_preserves_leading_zeros() {
        // A cert number like 0012345 must survive the round trip. Spreadsheets
        // strip leading zeros from unquoted numerics.
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir.path().join("vault"));
        v.conn()
            .execute(
                "INSERT INTO assets (asset_id, type_id, name, notes, created_at, updated_at)
                 VALUES ('a1','generic','Graded Coin','0012345',?1,?1)",
                [NOW],
            )
            .unwrap();

        let exported = export_assets(&v, NOW).unwrap();
        assert!(exported.csv.contains("\"0012345\""), "leading-zero value was not quoted");
    }

    #[test]
    fn formula_looking_values_are_escaped_reversibly() {
        assert_eq!(escape_formula("=SUM(A1:A9)"), "'=SUM(A1:A9)");
        assert_eq!(escape_formula("+1234"), "'+1234");
        assert_eq!(escape_formula("@user"), "'@user");
        assert_eq!(escape_formula("normal"), "normal");

        // And the escape is undone on the way back in.
        assert_eq!(unescape_formula("'=SUM(A1:A9)"), "=SUM(A1:A9)");
        assert_eq!(unescape_formula("normal"), "normal");
        // An apostrophe that is part of the data survives.
        assert_eq!(unescape_formula("'tis a name"), "'tis a name");
    }

    #[test]
    fn round_trip_preserves_values() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir.path().join("vault"));
        v.conn()
            .execute(
                "INSERT INTO assets
                   (asset_id, type_id, name, quantity, notes, acquired_amount_minor,
                    acquired_currency, created_at, updated_at)
                 VALUES ('a1','gold_bullion','1 oz Maple','2','0098765',129900,'USD',?1,?1)",
                [NOW],
            )
            .unwrap();

        let exported = export_assets(&v, NOW).unwrap();
        assert_eq!(exported.row_count, 1);

        // Re-import the unmodified export: nothing should change.
        let later = "2026-09-20T00:00:00Z";
        let preview = import_assets(&v, &exported.csv, ImportMode::Apply, later).unwrap();
        assert_eq!(preview.updates, 1);
        assert!(preview.errors.is_empty());

        let (name, qty, notes, amount): (String, String, String, i64) = v
            .conn()
            .query_row(
                "SELECT name, quantity, notes, acquired_amount_minor FROM assets WHERE asset_id='a1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(name, "1 oz Maple");
        assert_eq!(qty, "2");
        assert_eq!(notes, "0098765", "leading zeros lost in the round trip");
        assert_eq!(amount, 129900);
    }

    #[test]
    fn reimport_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir.path().join("vault"));
        insert(&v, "a1", "Original");

        let exported = export_assets(&v, NOW).unwrap();
        for _ in 0..3 {
            import_assets(&v, &exported.csv, ImportMode::Apply, "2026-09-21T00:00:00Z").unwrap();
        }

        let count: i64 =
            v.conn().query_row("SELECT count(*) FROM assets", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1, "repeated import duplicated rows");

        let events: i64 = v
            .conn()
            .query_row("SELECT count(*) FROM asset_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 0, "updates must not append acquisition events");
    }

    #[test]
    fn blank_preserves_and_dash_clears() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir.path().join("vault"));
        v.conn()
            .execute(
                "INSERT INTO assets (asset_id, type_id, name, notes, storage_location,
                                     created_at, updated_at)
                 VALUES ('a1','generic','Keep','important note','Safe 1',?1,?1)",
                [NOW],
            )
            .unwrap();

        // notes blank (preserve), storage_location "-" (clear).
        let csv = "asset_id,name,notes,storage_location\na1,Keep,,-\n";
        import_assets(&v, csv, ImportMode::Apply, "2026-09-21T00:00:00Z").unwrap();

        let (notes, location): (String, Option<String>) = v
            .conn()
            .query_row(
                "SELECT notes, storage_location FROM assets WHERE asset_id='a1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(notes, "important note", "a blank cell must not wipe stored data");
        assert_eq!(location, None, "the clear marker must clear the field");
    }

    #[test]
    fn preview_reports_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir.path().join("vault"));
        insert(&v, "a1", "Existing");

        let csv = "asset_id,name\na1,Renamed\n,Brand New\n";
        let preview = import_assets(&v, csv, ImportMode::Preview, NOW).unwrap();

        assert_eq!(preview.updates, 1);
        assert_eq!(preview.creates, 1);

        let name: String = v
            .conn()
            .query_row("SELECT name FROM assets WHERE asset_id='a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "Existing", "preview must not write");
    }

    #[test]
    fn errors_are_collected_not_thrown_one_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir.path().join("vault"));

        let csv = "asset_id,name,quantity,current_amount_minor,current_currency\n\
                   ,A,notanumber,,\n\
                   ,B,1,5000,\n\
                   ,C,1,notanumber,USD\n";
        let preview = import_assets(&v, csv, ImportMode::Preview, NOW).unwrap();

        assert!(preview.errors.len() >= 3, "expected several errors, got {:?}", preview.errors);
        assert!(preview.errors.iter().any(|e| e.contains("quantity")));
        assert!(preview.errors.iter().any(|e| e.contains("needs current_currency")));
    }

    #[test]
    fn a_file_with_errors_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir.path().join("vault"));

        let csv = "asset_id,name,quantity\n,Good Row,1\n,Bad Row,notanumber\n";
        let preview = import_assets(&v, csv, ImportMode::Apply, NOW).unwrap();
        assert!(!preview.errors.is_empty());

        let count: i64 =
            v.conn().query_row("SELECT count(*) FROM assets", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0, "a file with any error must be all-or-nothing");
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir.path().join("vault"));
        insert(&v, "a1", "One");

        let csv = "asset_id,name\na1,First\na1,Second\n";
        let preview = import_assets(&v, csv, ImportMode::Preview, NOW).unwrap();
        assert!(preview.errors.iter().any(|e| e.contains("repeats")));
    }

    #[test]
    fn a_file_from_another_vault_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let a = vault(&dir.path().join("a"));
        let b = vault(&dir.path().join("b"));
        insert(&b, "b1", "Theirs");

        let theirs = export_assets(&b, NOW).unwrap();
        assert!(matches!(
            import_assets(&a, &theirs.csv, ImportMode::Preview, NOW),
            Err(CsvError::WrongVault { .. })
        ));
    }

    #[test]
    fn a_stale_export_is_refused() {
        // Export, edit in the app, then try to import the old spreadsheet.
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir.path().join("vault"));
        insert(&v, "a1", "Original");

        let exported = export_assets(&v, "2026-09-19T00:00:00Z").unwrap();

        v.conn()
            .execute(
                "UPDATE assets SET name='Edited in app', updated_at='2026-09-20T00:00:00Z'
                 WHERE asset_id='a1'",
                [],
            )
            .unwrap();

        assert!(
            matches!(
                import_assets(&v, &exported.csv, ImportMode::Apply, NOW),
                Err(CsvError::StaleExport { .. })
            ),
            "an outdated spreadsheet must not silently overwrite newer edits"
        );

        let name: String = v
            .conn()
            .query_row("SELECT name FROM assets WHERE asset_id='a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "Edited in app");
    }

    #[test]
    fn missing_required_columns_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir.path().join("vault"));
        assert!(matches!(
            import_assets(&v, "type_id,quantity\ngeneric,1\n", ImportMode::Preview, NOW),
            Err(CsvError::MissingColumn(_))
        ));
    }

    #[test]
    fn oversized_input_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir.path().join("vault"));
        let huge = "x".repeat(MAX_FILE_BYTES + 1);
        assert!(matches!(
            import_assets(&v, &huge, ImportMode::Preview, NOW),
            Err(CsvError::TooLarge)
        ));
    }

    #[test]
    fn new_rows_get_an_acquisition_event() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(&dir.path().join("vault"));

        import_assets(&v, "asset_id,name,quantity\n,Imported,3\n", ImportMode::Apply, NOW)
            .unwrap();

        let (kind, delta): (String, String) = v
            .conn()
            .query_row(
                "SELECT event_type, quantity_delta FROM asset_events",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "acquire");
        assert_eq!(delta, "3");
    }
}
