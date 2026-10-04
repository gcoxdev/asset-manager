//! Importing an ordinary spreadsheet: map its columns, preview every row,
//! then import all of it or none.
//!
//! The preview is the import itself, run inside a unit of work that is then
//! rolled back. So what the preview shows — every rejection, every derived
//! name, every amount — is exactly what applying would store; there is no
//! second set of checks to drift out of step with the add form.

use std::collections::{BTreeMap, HashMap};

use am_storage::spreadsheet::{self, DateOrder, Table};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::asset_commands::{create_from_form, AssetForm};
use crate::ipc::{bad_input, base_currency, format_money, now, storage, IpcResult};
use crate::session::{IpcError, Session, SessionError};

const MAPPINGS_SETTING: &str = "import_mappings";
/// Rows shown in the column-mapping step.
const SAMPLE_ROWS: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportOptions {
    pub date_order: DateOrder,
    /// "1.234,56" rather than "1,234.56".
    pub decimal_comma: bool,
    /// Currency for amounts in rows without a currency column. The base
    /// currency when absent.
    #[serde(default)]
    pub currency: Option<String>,
    /// The type for rows with no type, or a type the app does not recognize.
    pub default_type: String,
}

#[derive(Serialize)]
pub struct Inspected {
    pub headers: Vec<String>,
    pub samples: Vec<Vec<String>>,
    pub row_count: usize,
    pub delimiter: String,
    /// Per column: a field, `detail:<key>`, or `ignore`.
    pub mapping: Vec<String>,
    pub options: ImportOptions,
    /// True when this mapping was remembered from an earlier import of a
    /// file with the same columns.
    pub remembered: bool,
    pub fields: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Preset {
    mapping: Vec<String>,
    options: ImportOptions,
}

fn signature(headers: &[String]) -> String {
    headers.iter().map(|h| h.to_lowercase()).collect::<Vec<_>>().join("\u{1f}")
}

fn read(contents: &str) -> IpcResult<Table> {
    spreadsheet::read_table(contents).map_err(|e| bad_input(e.to_string()))
}

/// A first look: columns, a few rows, and a guess at what each column is.
#[tauri::command]
pub fn inspect_spreadsheet(
    session: State<'_, Session>,
    contents: String,
) -> IpcResult<Inspected> {
    session.touch();
    let table = read(&contents)?;
    session
        .with_vault(|vault| {
            let presets: HashMap<String, Preset> =
                am_storage::settings::get(vault, MAPPINGS_SETTING)
                    .ok()
                    .flatten()
                    .and_then(|j| serde_json::from_str(&j).ok())
                    .unwrap_or_default();
            let saved = presets
                .get(&signature(&table.headers))
                .filter(|p| p.mapping.len() == table.headers.len());
            let (mapping, options, remembered) = match saved {
                Some(p) => (p.mapping.clone(), p.options.clone(), true),
                None => {
                    let mapping = spreadsheet::guess_mapping(&table.headers);
                    let options = ImportOptions {
                        date_order: guess_date_order(&table, &mapping),
                        decimal_comma: table.delimiter == ";",
                        currency: Some(base_currency(vault).code().to_string()),
                        default_type: "generic".into(),
                    };
                    (mapping, options, false)
                }
            };
            Ok(Inspected {
                samples: table.rows.iter().take(SAMPLE_ROWS).cloned().collect(),
                row_count: table.rows.len(),
                delimiter: table.delimiter.clone(),
                headers: table.headers.clone(),
                mapping,
                options,
                remembered,
                fields: spreadsheet::FIELDS.iter().map(|f| f.to_string()).collect(),
            })
        })
        .map_err(IpcError::from)
}

/// Day-first or month-first, from the dates themselves where they settle
/// it: a first part over 12 can only be a day. Month-first otherwise.
fn guess_date_order(table: &Table, mapping: &[String]) -> DateOrder {
    let Some(col) = mapping.iter().position(|m| m == "acquired_date") else {
        return DateOrder::MonthDayYear;
    };
    for row in &table.rows {
        let parts: Vec<u32> = row
            .get(col)
            .map(|c| c.split(['/', '-', '.']).filter_map(|p| p.trim().parse().ok()).collect())
            .unwrap_or_default();
        if parts.len() == 3 {
            if parts[0] > 31 {
                return DateOrder::YearMonthDay;
            }
            if parts[0] > 12 {
                return DateOrder::DayMonthYear;
            }
            if parts[1] > 12 {
                return DateOrder::MonthDayYear;
            }
        }
    }
    DateOrder::MonthDayYear
}

#[derive(Serialize)]
pub struct RowResult {
    /// The line in the file, counting the header as line 1.
    pub line: usize,
    /// "ok", "error" or "skipped".
    pub status: String,
    pub name: Option<String>,
    pub type_label: Option<String>,
    pub quantity: Option<String>,
    pub paid: Option<String>,
    pub value: Option<String>,
    pub location: Option<String>,
    pub tags: Vec<String>,
    pub error: Option<String>,
    /// Things worth a look that do not stop the row: a possible duplicate, a
    /// type that was not recognized.
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
pub struct ImportReport {
    pub rows: Vec<RowResult>,
    pub ready: usize,
    pub errors: usize,
    pub skipped: usize,
    pub warnings: usize,
    /// True only when the import was applied — every row valid.
    pub applied: bool,
}

fn valid_target(target: &str) -> bool {
    target == "ignore"
        || spreadsheet::FIELDS.contains(&target)
        || target.strip_prefix("detail:").is_some_and(|k| {
            !k.is_empty() && k.len() <= 64 && k.chars().all(|c| c.is_alphanumeric() || c == '_')
        })
}

/// Build the add-form a row describes. Errors are the owner's to fix in the
/// mapping or the file; warnings are worth a look.
fn row_form(
    row: &[String],
    mapping: &[String],
    options: &ImportOptions,
    types: &[(String, String)],
    warnings: &mut Vec<String>,
) -> Result<AssetForm, String> {
    let mut form = AssetForm {
        type_id: options.default_type.clone(),
        currency: options.currency.clone(),
        pricing: Some("manual".into()),
        ..Default::default()
    };
    let mut notes: Vec<String> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    let mut attrs = BTreeMap::new();
    let number = |v: &str| spreadsheet::normalize_number(v, options.decimal_comma);
    for (target, cell) in mapping.iter().zip(row.iter()) {
        let value = cell.trim();
        if value.is_empty() || target == "ignore" {
            continue;
        }
        match target.as_str() {
            "name" => form.name = Some(value.to_string()),
            "type" => match spreadsheet::match_type(value, types) {
                Some(id) => form.type_id = id,
                None => warnings.push(format!(
                    "type {value:?} is not one the app knows — added as {}",
                    types
                        .iter()
                        .find(|(id, _)| *id == options.default_type)
                        .map(|(_, l)| l.as_str())
                        .unwrap_or("the default type")
                )),
            },
            "quantity" => form.quantity = Some(number(value)),
            "unit" => form.quantity_unit = Some(value.to_string()),
            "acquired_date" => {
                form.acquired_date =
                    Some(spreadsheet::normalize_date(value, options.date_order)?)
            }
            "acquired_price" => form.acquired_price = Some(number(value)),
            "current_value" => form.current_value = Some(number(value)),
            "insured_value" => form.insured_value = Some(number(value)),
            "acquired_from" => form.acquired_from = Some(value.to_string()),
            "storage_location" => form.storage_location = Some(value.to_string()),
            "notes" => notes.push(value.to_string()),
            "currency" => form.currency = Some(value.to_uppercase()),
            "tags" => tags.extend(
                value
                    .split([',', ';', '|'])
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(String::from),
            ),
            detail => {
                if let Some(key) = detail.strip_prefix("detail:") {
                    attrs.insert(key.to_string(), value.to_string());
                }
            }
        }
    }
    form.notes = Some(notes.join("\n"));
    form.attrs = attrs;
    if !tags.is_empty() {
        form.tags = Some(tags);
    }
    Ok(form)
}

/// Preview — or, with `apply`, import — a spreadsheet under a mapping.
/// Applying imports every row not skipped, all together; it is refused while
/// any of them has an error.
#[tauri::command]
pub fn import_spreadsheet(
    session: State<'_, Session>,
    contents: String,
    mapping: Vec<String>,
    options: ImportOptions,
    skip: Vec<usize>,
    apply: bool,
) -> IpcResult<ImportReport> {
    session.touch();
    let timestamp = now();
    let table = read(&contents)?;
    if mapping.len() != table.headers.len() {
        return Err(bad_input("the mapping does not match the file's columns"));
    }
    if let Some(bad) = mapping.iter().find(|m| !valid_target(m)) {
        return Err(bad_input(format!("{bad:?} is not something a column can be imported as")));
    }

    session
        .with_vault(|vault| {
            let types: Vec<(String, String)> = am_storage::assets::types(vault)
                .map_err(storage)?
                .into_iter()
                .map(|t| (t.type_id, t.label))
                .collect();
            if !types.iter().any(|(id, _)| *id == options.default_type) {
                return Err(storage(format!("unknown type {:?}", options.default_type)));
            }

            // What is already in the catalog, to point out likely duplicates.
            let existing = am_storage::assets::list(vault).map_err(storage)?;
            let by_name: HashMap<String, String> =
                existing.iter().map(|a| (a.name.to_lowercase(), a.name.clone())).collect();
            let by_serial: HashMap<String, String> = existing
                .iter()
                .filter_map(|a| {
                    a.attrs
                        .get("serial_number")
                        .or_else(|| a.attrs.get("cert_number"))
                        .map(|s| (s.to_lowercase(), a.name.clone()))
                })
                .collect();
            let mut seen_in_file: HashMap<String, usize> = HashMap::new();

            let outer = am_storage::atomic::begin(vault.conn()).map_err(storage)?;
            let mut rows = Vec::with_capacity(table.rows.len());
            for (index, row) in table.rows.iter().enumerate() {
                let line = index + 2;
                let mut result = RowResult {
                    line,
                    status: "ok".into(),
                    name: None,
                    type_label: None,
                    quantity: None,
                    paid: None,
                    value: None,
                    location: None,
                    tags: Vec::new(),
                    error: None,
                    warnings: Vec::new(),
                };
                if skip.contains(&index) {
                    result.status = "skipped".into();
                    rows.push(result);
                    continue;
                }
                let form = match row_form(row, &mapping, &options, &types, &mut result.warnings)
                {
                    Ok(form) => form,
                    Err(e) => {
                        result.status = "error".into();
                        result.error = Some(e);
                        rows.push(result);
                        continue;
                    }
                };

                // Each row in its own unit inside the import's: a bad row
                // undoes only itself, so every problem is reported at once.
                let row_unit = am_storage::atomic::begin(vault.conn()).map_err(storage)?;
                match create_from_form(vault, &form, &timestamp) {
                    Ok(id) => {
                        let r = am_storage::assets::get(vault, &id).map_err(storage)?;
                        row_unit.commit().map_err(storage)?;
                        if let Some(original) = by_name.get(&r.name.to_lowercase()) {
                            result
                                .warnings
                                .push(format!("already in the catalog as {original:?}?"));
                        }
                        let serial =
                            r.attrs.get("serial_number").or_else(|| r.attrs.get("cert_number"));
                        if let Some(original) =
                            serial.and_then(|s| by_serial.get(&s.to_lowercase()))
                        {
                            result
                                .warnings
                                .push(format!("same serial or certificate as {original:?}"));
                        }
                        let key = format!(
                            "{}\u{1f}{}",
                            r.name.to_lowercase(),
                            serial.map(|s| s.to_lowercase()).unwrap_or_default()
                        );
                        if let Some(first) = seen_in_file.insert(key, line) {
                            result.warnings.push(format!("looks the same as line {first}"));
                        }
                        result.name = Some(r.name);
                        result.type_label = Some(r.type_label);
                        result.quantity = Some(format!("{} {}", r.quantity, r.quantity_unit));
                        result.paid = format_money(
                            r.acquired_amount_minor,
                            r.acquired_currency.as_deref(),
                        );
                        result.value =
                            format_money(r.current_amount_minor, r.current_currency.as_deref());
                        result.location = r.storage_location;
                        result.tags = r.tags;
                    }
                    Err(e) => {
                        drop(row_unit);
                        result.status = "error".into();
                        result.name = form.name.clone();
                        result.error = Some(IpcError::from(e).message);
                    }
                }
                rows.push(result);
            }

            let count = |s: &str| rows.iter().filter(|r| r.status == s).count();
            let (ready, errors, skipped) = (count("ok"), count("error"), count("skipped"));
            let warnings = rows.iter().filter(|r| !r.warnings.is_empty()).count();
            let applied = apply && errors == 0 && ready > 0;
            if applied {
                outer.commit().map_err(storage)?;
                // Remember how this file's columns were read, inside the
                // vault, for the next file laid out the same way.
                let mut presets: HashMap<String, Preset> =
                    am_storage::settings::get(vault, MAPPINGS_SETTING)
                        .ok()
                        .flatten()
                        .and_then(|j| serde_json::from_str(&j).ok())
                        .unwrap_or_default();
                if presets.len() >= 20 {
                    presets.clear();
                }
                presets.insert(
                    signature(&table.headers),
                    Preset { mapping: mapping.clone(), options: options.clone() },
                );
                if let Ok(json) = serde_json::to_string(&presets) {
                    let _ = am_storage::settings::set(vault, MAPPINGS_SETTING, &json);
                }
            }
            // Otherwise the unit is dropped here and every row rolls back:
            // a preview writes nothing.
            Ok(ImportReport { rows, ready, errors, skipped, warnings, applied })
        })
        .map_err(|e: SessionError| IpcError::from(e))
}
