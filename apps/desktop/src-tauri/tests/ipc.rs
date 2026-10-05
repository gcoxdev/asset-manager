//! End-to-end IPC test on Tauri's mock runtime.
//!
//! Drives the real command table through real IPC serialization, with the
//! argument shapes the frontend sends. The storage layer has its own unit
//! tests; what this catches is the seam between them — a camelCase argument
//! that does not match its Rust name, a struct field the UI reads that the
//! backend renamed, a command that was never registered. Those fail at
//! runtime in the app and nowhere else.
//!
//! One test function, deliberately: the vault location comes from an
//! environment variable, and parallel tests would race on it.

use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::WebviewWindow;

const PASS: &str = "correct horse battery staple";

fn invoke(
    webview: &WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    args: Value,
) -> Result<Value, Value> {
    get_ipc_response(
        webview,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize::<Value>().unwrap())
}

/// Invoke a command that answers with raw bytes (an ArrayBuffer in the UI).
fn raw(webview: &WebviewWindow<tauri::test::MockRuntime>, cmd: &str, args: Value) -> Vec<u8> {
    let body = get_ipc_response(
        webview,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .unwrap_or_else(|e| panic!("{cmd} failed: {e}"));
    match body {
        tauri::ipc::InvokeResponseBody::Raw(bytes) => bytes,
        other => panic!("{cmd} answered with JSON, not bytes: {other:?}"),
    }
}

/// Invoke and insist on success, with the error in the panic message.
fn ok(webview: &WebviewWindow<tauri::test::MockRuntime>, cmd: &str, args: Value) -> Value {
    invoke(webview, cmd, args).unwrap_or_else(|e| panic!("{cmd} failed: {e}"))
}

#[test]
fn the_frontend_contract_holds_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    // Debug builds honour this override; release builds never read it.
    std::env::set_var("AM_VAULT_DIR", &vault);

    let app = asset_manager_desktop_lib::configure(mock_builder())
        .build(mock_context(noop_assets()))
        .expect("app builds");
    let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();

    // --- vault lifecycle ---------------------------------------------------
    assert_eq!(
        ok(&w, "vault_status", json!({})),
        json!({ "unlocked": false, "exists": false })
    );
    assert!(
        invoke(&w, "list_assets", json!({})).is_err(),
        "nothing is readable before a vault exists"
    );

    let weak = invoke(&w, "create_vault", json!({ "passphrase": "short" })).unwrap_err();
    assert_eq!(weak["kind"], "weak_passphrase");
    let created = ok(&w, "create_vault", json!({ "passphrase": PASS }));
    let recovery = created["recovery_key"].as_str().unwrap().to_string();
    assert!(!created["fingerprint"].as_str().unwrap().is_empty());

    // --- settings ----------------------------------------------------------
    let settings = ok(&w, "get_settings", json!({}));
    assert_eq!(settings["currency"], "USD");
    assert_eq!(settings["recovery_unconfirmed"], true, "until the ceremony finishes");
    ok(&w, "confirm_recovery_saved", json!({}));
    assert_eq!(ok(&w, "get_settings", json!({}))["recovery_unconfirmed"], false);
    ok(
        &w,
        "update_settings",
        json!({ "settings": {
            "currency": "USD", "auto_lock_minutes": 30, "metals_auto_refresh": false,
            "balance_lookup": false, "last_backup_at": null
        }}),
    );
    assert_eq!(ok(&w, "session_state", json!({}))["auto_lock_seconds"], 1800);

    // --- a market-priced metal holding --------------------------------------
    let eagles = ok(
        &w,
        "create_asset",
        json!({ "form": {
            "type_id": "sovereign_coin", "name": "Gold Eagles", "quantity": "10", "quantity_unit": "coin",
            "acquired_date": "2024-01-02", "acquired_price": "$20,000.00", "acquired_from": null,
            "storage_location": "Safe", "notes": "", "insured_value": null, "currency": "USD",
            "attrs": { "metal": "XAU", "weight_per_item": "1.0909", "weight_unit": "troy_oz",
                       "weight_basis": "gross", "purity": "0.9167", "preset": "age" },
            "review_every_days": null, "pricing": "market", "current_value": null
        }}),
    );
    let eagles = eagles.as_str().unwrap().to_string();

    // No spot price yet: counted, never zero.
    let dash = ok(&w, "dashboard", json!({}));
    assert_eq!(dash["unvalued"], 1);
    assert_eq!(dash["total"]["minor"], "0");

    let revalued = ok(
        &w,
        "set_spot_price",
        json!({ "metal": "XAU", "price": "2,000", "currency": "USD" }),
    );
    assert_eq!(revalued["updated"], 1, "a spot price must revalue the holding that follows it");

    let detail = ok(&w, "get_asset", json!({ "assetId": eagles }));
    // 10 × 1.0909 × 0.9167 = 10.0002803 fine oz × $2,000
    assert_eq!(detail["asset"]["current_display"], "20000.56 USD");
    assert_eq!(detail["asset"]["value_source"], "manual", "valued from a hand-typed spot");
    assert_eq!(detail["asset"]["gain_display"], "0.56 USD");
    assert_eq!(detail["market"]["unit_price"], "2000");
    assert_eq!(detail["events"][0]["effective_date"], "2024-01-02");

    // --- a collectible, named from its fields --------------------------------
    let preview = ok(
        &w,
        "validate_asset",
        json!({ "form": {
            "type_id": "comic", "name": null,
            "attrs": { "title": "Amazing Fantasy", "issue": "15", "grader": "cgc", "grade": "9.8" }
        }}),
    );
    assert_eq!(preview, "Amazing Fantasy #15 — CGC 9.8");
    let bad_grade = invoke(&w, "create_asset", json!({ "form": {
        "type_id": "comic", "attrs": { "title": "X", "issue": "1", "grader": "psa", "grade": "65" }
    }}))
    .unwrap_err();
    assert!(bad_grade["message"].as_str().unwrap().contains("PSA"));

    let comic = ok(
        &w,
        "create_asset",
        json!({ "form": {
            "type_id": "comic", "name": null, "quantity": "1", "quantity_unit": "item",
            "attrs": { "title": "Amazing Fantasy", "issue": "15", "grader": "cgc", "grade": "9.8",
                       "cert_number": "0012345678" },
            "pricing": "manual", "current_value": "1,250,000", "review_every_days": 90
        }}),
    );
    let comic = comic.as_str().unwrap().to_string();

    let list = ok(&w, "list_assets", json!({}));
    let row =
        list.as_array().unwrap().iter().find(|a| a["asset_id"] == comic.as_str()).unwrap();
    assert_eq!(row["type_id"], "comic", "collectibles keep their type");
    assert_eq!(row["category"], "collectibles");
    assert_eq!(row["current_display"], "1250000.00 USD");
    assert_eq!(row["current_amount_minor"], "125000000", "money crosses IPC as text");
    assert_eq!(row["next_review"].as_str().map(|d| d.len()), Some(10));

    // Firearms are their own category, found by serial number.
    let types = ok(&w, "asset_types", json!({}));
    assert!(types
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["type_id"] == "firearm" && t["category"] == "firearms"));
    let rifle = ok(
        &w,
        "create_asset",
        json!({ "form": {
            "type_id": "firearm", "name": "Model 70", "quantity": "1", "quantity_unit": "item",
            "attrs": { "manufacturer": "Winchester", "caliber": ".30-06", "serial_number": "G1234567" },
            "pricing": "manual", "current_value": null
        }}),
    );
    let rifle = rifle.as_str().unwrap().to_string();
    assert_eq!(
        ok(&w, "get_asset", json!({ "assetId": rifle }))["asset"]["category"],
        "firearms"
    );
    assert_eq!(ok(&w, "search_assets", json!({ "query": "G1234567" })), json!([rifle.clone()]));
    ok(&w, "delete_asset", json!({ "assetId": rifle }));
    ok(&w, "purge_trash", json!({ "assetId": rifle }));

    // Care: a service with its cost, and the next one due soon.
    let care_id = ok(
        &w,
        "add_care",
        json!({ "entry": { "asset_id": comic, "kind": "inspection", "performed_on": "2026-01-02",
                           "provider": "CGC", "cost": "45", "next_due": "2026-01-03" } }),
    );
    let history = ok(&w, "list_care", json!({ "assetId": comic }));
    assert_eq!(history[0]["cost_display"], "45.00 USD");
    let due = ok(&w, "care_due", json!({}));
    assert_eq!(due[0]["overdue"], true);
    assert_eq!(due[0]["asset_name"], "Amazing Fantasy #15 — CGC 9.8");
    ok(&w, "delete_care", json!({ "careId": care_id }));
    assert_eq!(ok(&w, "care_due", json!({})), json!([]));

    // Custody: consigned, shown as away, then back.
    ok(
        &w,
        "record_custody",
        json!({ "entry": { "asset_id": comic, "kind": "consigned", "party": "Heritage",
                           "contact": "consign@example.com", "date": "2026-01-05",
                           "due_back": "2026-02-01", "reference": "C-123" } }),
    );
    let away = ok(&w, "away_list", json!({}));
    assert_eq!(away[0]["overdue"], true);
    let row = ok(&w, "list_assets", json!({}));
    let row = row
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["asset_id"] == comic.as_str())
        .unwrap()
        .clone();
    assert_eq!(
        (row["away"].as_str(), row["away_with"].as_str()),
        (Some("consigned"), Some("Heritage"))
    );
    ok(
        &w,
        "record_custody",
        json!({ "entry": { "asset_id": comic, "kind": "returned", "date": "2026-02-02" } }),
    );
    assert_eq!(ok(&w, "away_list", json!({})), json!([]));
    let csv = ok(&w, "export_csv", json!({}));
    assert!(
        !csv["csv"].as_str().unwrap().contains("consign@example.com"),
        "contacts never leave"
    );

    // Another currency: listed apart until a rate is recorded, then counted.
    let euro = ok(
        &w,
        "create_asset",
        json!({ "form": { "type_id": "art", "name": "Paris print", "current_value": "1000",
                          "currency": "EUR" } }),
    );
    let before = ok(&w, "dashboard", json!({}));
    assert!(before["skipped_currencies"].as_array().unwrap().iter().any(|c| c == "EUR"));
    let rate = ok(
        &w,
        "record_rate",
        json!({ "from": "EUR", "to": "USD", "rate": "1.10", "asof": null }),
    );
    let after = ok(&w, "dashboard", json!({}));
    assert_eq!(after["converted"][0]["from_currency"], "EUR");
    let listed = ok(&w, "list_assets", json!({}));
    let row = listed.as_array().unwrap().iter().find(|a| a["asset_id"] == euro).unwrap();
    assert_eq!(row["value_in_base_display"], "1100.00 USD");
    assert_eq!(row["current_display"], "1000.00 EUR", "the original is never rewritten");
    ok(&w, "delete_rate", json!({ "rateId": rate }));
    ok(&w, "delete_asset", json!({ "assetId": euro }));
    ok(&w, "purge_trash", json!({ "assetId": euro }));

    // A wish is never owned; buying it records the asset it became.
    let wish = ok(
        &w,
        "save_wish",
        json!({ "form": { "name": "Daytona", "type_id": "watch", "target": "15,000", "priority": "high" } }),
    );
    let wishes = ok(&w, "list_wishes", json!({}));
    assert_eq!(wishes[0]["target_display"], "15000.00 USD");
    let bought = ok(
        &w,
        "create_asset",
        json!({ "form": { "type_id": "watch", "name": "Daytona", "acquired_price": "14500" } }),
    );
    ok(&w, "wish_acquired", json!({ "wishId": wish, "assetId": bought }));
    assert_eq!(ok(&w, "list_wishes", json!({}))[0]["acquired_asset_id"], bought);
    ok(&w, "delete_wish", json!({ "wishId": wish }));
    ok(&w, "delete_asset", json!({ "assetId": bought }));
    ok(&w, "purge_trash", json!({ "assetId": bought }));

    // An inventory check of the safe, with a scanned label.
    let check = ok(&w, "start_check", json!({ "name": "Safe", "location": "Safe" }));
    let in_scope = ok(&w, "check_items", json!({ "checkId": check }));
    assert!(in_scope
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["storage_location"].as_str().unwrap().starts_with("Safe")));
    let labels = ok(&w, "label_data", json!({ "assetIds": [eagles] }));
    assert_eq!(labels[0]["payload"], format!("AM:{eagles}"), "the label is the opaque ID only");
    let found = ok(&w, "resolve_label", json!({ "code": labels[0]["payload"] }));
    assert_eq!(found, eagles.as_str());
    ok(&w, "mark_item", json!({ "checkId": check, "assetId": eagles, "result": "present" }));
    ok(&w, "finish_check", json!({ "checkId": check }));
    let row = ok(&w, "list_assets", json!({}));
    let row = row
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["asset_id"] == eagles.as_str())
        .unwrap()
        .clone();
    assert!(row["last_seen"].as_str().is_some(), "found in a check");
    ok(&w, "delete_check", json!({ "checkId": check }));

    // A type of the owner's own, validated like the built-in ones.
    let quilt = ok(
        &w,
        "save_custom_type",
        json!({ "definition": { "label": "Quilt", "category": "collectibles", "fields": [
            { "label": "Maker", "kind": "text", "required": true },
            { "label": "Pattern", "kind": "choice", "options": ["Log cabin", "Star"], "required": false } ] } }),
    );
    assert_eq!(quilt, "custom_quilt");
    let missing = invoke(
        &w,
        "create_asset",
        json!({ "form": { "type_id": "custom_quilt", "name": "Wedding quilt" } }),
    )
    .unwrap_err();
    assert!(missing["message"].as_str().unwrap().contains("Maker is required"));
    let made = ok(
        &w,
        "create_asset",
        json!({ "form": { "type_id": "custom_quilt", "name": "Wedding quilt",
                          "attrs": { "maker": "Grandma", "pattern": "star" } } }),
    );
    let made_detail = ok(&w, "get_asset", json!({ "assetId": made }));
    assert_eq!(made_detail["asset"]["attrs"]["pattern"], "Star");
    assert_eq!(made_detail["asset"]["type_label"], "Quilt");
    assert!(
        invoke(&w, "delete_custom_type", json!({ "typeId": "custom_quilt" })).is_err(),
        "in use"
    );
    ok(&w, "delete_asset", json!({ "assetId": made }));
    ok(&w, "purge_trash", json!({ "assetId": made }));
    ok(&w, "delete_custom_type", json!({ "typeId": "custom_quilt" }));

    // Organizing: tags, a location move, a bulk change, a duplicate, a view.
    ok(&w, "set_asset_tags", json!({ "assetId": comic, "tags": ["Key issues", "Insured"] }));
    assert_eq!(ok(&w, "list_tags", json!({})).as_array().unwrap().len(), 2);
    let changed = ok(
        &w,
        "bulk_edit",
        json!({ "assetIds": [comic, eagles], "change": {
            "storage_location": "Safe / Top shelf", "add_tags": ["Vault 1"] } }),
    );
    assert_eq!(changed, 2);
    assert_eq!(ok(&w, "rename_location", json!({ "from": "Safe", "to": "Bank box" })), 2);
    let detail = ok(&w, "get_asset", json!({ "assetId": eagles }));
    assert_eq!(detail["asset"]["storage_location"], "Bank box / Top shelf");
    assert_eq!(detail["asset"]["tags"], json!(["Vault 1"]));
    let copy = ok(&w, "duplicate_asset", json!({ "assetId": comic }));
    let copied = ok(&w, "get_asset", json!({ "assetId": copy }));
    assert!(copied["asset"]["attrs"]["cert_number"].is_null(), "identifiers are not copied");
    ok(&w, "bulk_trash", json!({ "assetIds": [copy] }));
    ok(&w, "purge_trash", json!({ "assetId": copy }));
    ok(&w, "save_view", json!({ "name": "In the bank", "view": { "location": "Bank box" } }));
    assert_eq!(ok(&w, "list_saved_views", json!({}))[0]["name"], "In the bank");
    ok(&w, "delete_saved_view", json!({ "name": "In the bank" }));
    // Put the location back for the checks below.
    ok(&w, "rename_location", json!({ "from": "Bank box", "to": "Safe" }));

    // Search: punctuation is harmless and attributes are indexed.
    assert_eq!(ok(&w, "search_assets", json!({ "query": "#15 cgc" })), json!([comic.clone()]));
    assert_eq!(
        ok(&w, "search_assets", json!({ "query": "0012345678" })),
        json!([comic.clone()])
    );

    // --- valuation precedence and quantity changes ---------------------------
    let results = ok(
        &w,
        "set_prices",
        json!({ "entries": [
            { "asset_id": eagles, "amount": "25000", "currency": "USD", "asof": null,
              "basis": "estimated_resale", "provenance": "manual", "note": "dealer quote" }
        ]}),
    );
    assert_eq!(results[0]["ok"], true, "{results}");
    // A value with its reasons; a range that excludes the value is refused.
    let explained = ok(
        &w,
        "set_prices",
        json!({ "entries": [
            { "asset_id": comic, "amount": "1,250,000", "currency": "USD", "asof": null,
              "basis": "estimated_resale", "provenance": "manual", "note": null,
              "evidence": { "comparables": [
                  { "description": "CGC 9.8, Heritage", "price": "1,200,000", "kind": "sold", "date": "2026-06-01" },
                  { "description": "CGC 9.8, eBay", "price": "1,400,000", "kind": "asking" } ],
                "low": "1,100,000", "high": "1,400,000", "confidence": "medium" } },
            { "asset_id": eagles, "amount": "25000", "currency": "USD", "asof": null,
              "evidence": { "low": "26000" } }
        ]}),
    );
    assert_eq!(explained[0]["ok"], true, "{explained}");
    assert_eq!(explained[1]["ok"], false, "the value is outside its own range");
    let why = ok(&w, "get_asset", json!({ "assetId": comic }));
    let evidence = &why["valuations"][0]["evidence"];
    assert_eq!(evidence["comparables"][0]["price"], "1200000.00 USD");
    assert_eq!(evidence["range"], "1100000.00 USD – 1400000.00 USD");
    assert_eq!(evidence["confidence"], "medium");

    let detail = ok(&w, "get_asset", json!({ "assetId": eagles }));
    assert_eq!(detail["asset"]["pricing"], "manual", "a typed price stops market tracking");
    assert_eq!(detail["valuations"][0]["note"], "dealer quote");

    ok(
        &w,
        "change_quantity",
        json!({ "change": {
            "asset_id": eagles, "kind": "correct", "quantity": "8", "effective_date": null,
            "amount": null, "currency": "USD", "note": null
        }}),
    );
    let detail = ok(&w, "get_asset", json!({ "assetId": eagles }));
    assert_eq!(detail["asset"]["quantity"], "8", "a correction can reduce the count");
    assert_eq!(detail["asset"]["current_display"], "20000.00 USD", "8 of 10: value scales");

    ok(&w, "set_pricing", json!({ "assetId": eagles, "pricing": "market" }));
    let detail = ok(&w, "get_asset", json!({ "assetId": eagles }));
    assert_eq!(detail["asset"]["current_display"], "16000.45 USD", "back on spot, at 8 coins");

    // --- editing --------------------------------------------------------------
    ok(
        &w,
        "update_asset",
        json!({ "assetId": comic, "form": {
            "type_id": "comic", "name": "AF15", "status": "lost", "status_date": "2026-09-01",
            "quantity_unit": "item",
            "acquired_date": "2010-05-01", "acquired_price": "1100", "storage_location": "",
            "notes": "Reported to insurer", "insured_value": "1300000", "currency": "USD",
            "attrs": { "title": "Amazing Fantasy", "issue": "15", "grader": "cgc", "grade": "9.8" },
            "review_every_days": null
        }}),
    );
    let detail = ok(&w, "get_asset", json!({ "assetId": comic }));
    assert_eq!(detail["asset"]["name"], "AF15");
    assert_eq!(detail["asset"]["status"], "lost");
    assert_eq!(detail["asset"]["insured_display"], "1300000.00 USD");
    assert_eq!(detail["asset"]["storage_location"], Value::Null, "blank clears");

    // Changing the base currency relabels nothing. A notes-only edit sent in
    // EUR keeps the stored USD cost, and each amount keeps its own currency.
    let settings_in = |currency: &str| {
        json!({ "settings": {
            "currency": currency, "auto_lock_minutes": 30, "metals_auto_refresh": false,
            "balance_lookup": false
        }})
    };
    ok(&w, "update_settings", settings_in("EUR"));
    let eagles_form = |notes: &str, insured_currency: Option<&str>| {
        json!({ "assetId": eagles, "form": {
            "type_id": "sovereign_coin", "name": "Gold Eagles", "quantity_unit": "coin",
            "acquired_date": "2024-01-02", "acquired_price": "20000.00",
            "storage_location": "Safe", "notes": notes, "insured_value": "15000",
            "insured_currency": insured_currency, "currency": "EUR",
            "attrs": { "metal": "XAU", "weight_per_item": "1.0909", "weight_unit": "troy_oz",
                       "weight_basis": "gross", "purity": "0.9167", "preset": "age" },
            "review_every_days": null
        }})
    };
    ok(&w, "update_asset", eagles_form("insured separately", Some("CHF")));
    let detail = ok(&w, "get_asset", json!({ "assetId": eagles }));
    assert_eq!(detail["asset"]["acquired_display"], "20000.00 USD", "cost keeps its currency");
    assert_eq!(detail["asset"]["insured_display"], "15000.00 CHF");
    ok(&w, "update_asset", eagles_form("notes only", None));
    let detail = ok(&w, "get_asset", json!({ "assetId": eagles }));
    assert_eq!(detail["asset"]["acquired_display"], "20000.00 USD");
    assert_eq!(detail["asset"]["insured_display"], "15000.00 CHF", "stored currency wins");
    ok(&w, "update_settings", settings_in("USD"));

    // Split part of a holding off; put both in a set.
    let part = ok(
        &w,
        "split_asset",
        json!({ "assetId": eagles, "quantity": "2", "name": "Gold Eagles — pair" }),
    );
    let pair = ok(&w, "get_asset", json!({ "assetId": part }));
    assert_eq!(pair["asset"]["quantity"], "2");
    let rest = ok(&w, "get_asset", json!({ "assetId": eagles }));
    assert_eq!(rest["events"][0]["event_type"], "split");
    let set = ok(
        &w,
        "save_set",
        json!({ "name": "Eagles", "targetCount": 10, "add": [eagles, part] }),
    );
    assert_eq!(ok(&w, "list_sets", json!({}))[0]["members"], 2);
    assert_eq!(ok(&w, "get_asset", json!({ "assetId": part }))["sets"][0][1], "Eagles");
    let shares = ok(
        &w,
        "allocate_purchase",
        json!({ "assetIds": [eagles, part], "total": "100.01", "byValue": false }),
    );
    assert_eq!(shares[0][1], "50.01 USD");
    ok(&w, "delete_set", json!({ "setId": set }));
    ok(&w, "delete_asset", json!({ "assetId": part }));
    ok(&w, "purge_trash", json!({ "assetId": part }));

    // --- reports and charts -----------------------------------------------------
    let report = ok(
        &w,
        "insurance_report",
        json!({ "options": {
            "include_locations": false, "include_notes": false, "include_photos": true
        }}),
    );
    assert_eq!(report["items"].as_array().unwrap().len(), 1, "a lost item is not claimed");
    // …unless asked for, for a claim — with the date it was lost.
    let claim = ok(&w, "insurance_report", json!({ "options": { "include_lost": true } }));
    assert_eq!(claim["items"].as_array().unwrap().len(), 2);
    assert_eq!(claim["lost"], 1);
    let lost =
        claim["items"].as_array().unwrap().iter().find(|i| i["status"] == "lost").unwrap();
    assert_eq!(lost["lost_on"], "2026-09-01");
    assert_eq!(lost["current"], "1250000.00 USD", "its value from before the loss");
    // …with the evidence behind that value, printed by default.
    assert_eq!(lost["evidence"]["confidence"], "medium");
    assert_eq!(lost["evidence"]["comparables"].as_array().unwrap().len(), 2);
    let bare = ok(
        &w,
        "insurance_report",
        json!({ "options": { "include_lost": true, "include_evidence": false } }),
    );
    assert!(bare["items"].as_array().unwrap().iter().all(|i| i["evidence"].is_null()));
    let detail = ok(&w, "get_asset", json!({ "assetId": comic }));
    assert_eq!(detail["status_events"][0]["status"], "lost");

    // A claim packet: exactly the chosen item, as it stood before the loss.
    let packet = ok(
        &w,
        "insurance_report",
        json!({ "options": { "asset_ids": [comic], "as_of": "2026-08-31",
                             "compare_bases": true, "include_documents": true } }),
    );
    assert_eq!(packet["items"].as_array().unwrap().len(), 1, "nothing else is disclosed");
    assert_eq!(packet["as_of"], "2026-08-31");
    // Its only value was recorded later, so on that date it had none —
    // unknown, not today's figure passed off as an earlier one.
    assert_eq!(packet["items"][0]["current"], Value::Null);
    let today = ok(
        &w,
        "insurance_report",
        json!({ "options": { "asset_ids": [comic], "compare_bases": true } }),
    );
    assert_eq!(today["items"][0]["values_by_basis"][0]["basis"], "estimated_resale");
    let claim_dir = dir.path().join("claims");
    std::fs::create_dir_all(&claim_dir).unwrap();
    let files = ok(
        &w,
        "export_claim_files",
        json!({ "assetIds": [comic], "directory": claim_dir.to_str().unwrap(), "includePhotos": true }),
    );
    assert!(files["folder"].as_str().unwrap().contains("Claim files"));

    // The edit can be undone from its history, and a mistyped value voided.
    let revisions = ok(&w, "asset_revisions", json!({ "assetId": comic }));
    assert_eq!(revisions[0]["snapshot"]["status"], "active");
    for valuation in detail["valuations"].as_array().unwrap() {
        ok(
            &w,
            "void_valuation",
            json!({ "valuationId": valuation["valuation_id"], "reason": "typo" }),
        );
    }
    let voided = ok(&w, "get_asset", json!({ "assetId": comic }));
    assert_eq!(voided["valuations"][0]["void_reason"], "typo");
    assert_eq!(voided["asset"]["current_display"], Value::Null, "unknown, not zero");
    assert_eq!(report["items"][0]["storage_location"], Value::Null, "locations left out");

    let series = ok(&w, "portfolio_series", json!({ "from": null, "maxPoints": 160 }));
    assert!(series["points"].as_array().unwrap().len() >= 2);

    let dash = ok(&w, "dashboard", json!({}));
    assert_eq!(dash["active_count"], 1);
    assert_eq!(dash["by_category"][0]["category"], "metals");

    let csv = ok(&w, "export_csv", json!({}));
    assert_eq!(csv["row_count"], 2);
    let preview = ok(&w, "import_csv", json!({ "contents": csv["csv"], "apply": false }));
    assert_eq!(preview["updates"], 2);
    assert_eq!(preview["errors"], json!([]));

    assert!(ok(&w, "bullion_presets", json!({})).as_array().unwrap().len() >= 10);
    assert!(ok(&w, "crypto_prices", json!({})).as_array().unwrap().is_empty());
    assert_eq!(ok(&w, "vault_info", json!({}))["asset_count"], 2);

    // --- crypto, priced by hand --------------------------------------------------
    let btc = ok(
        &w,
        "create_asset",
        json!({ "form": {
            "type_id": "crypto", "name": "Cold storage", "quantity": "0.12345678", "quantity_unit": "BTC",
            "attrs": { "coin_id": "bitcoin", "symbol": "BTC", "custody": "self_custody",
                       "watch_address": "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa", "watch_chain": "bitcoin" },
            "pricing": "market"
        }}),
    );
    let seed = invoke(
        &w,
        "create_asset",
        json!({ "form": {
            "type_id": "crypto", "name": "Oops", "attrs": { "coin_id": "bitcoin",
            "watch_address": "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about" }
        }}),
    );
    assert!(seed.is_err(), "a seed phrase must never be stored");
    let summary = ok(&w, "set_coin_price", json!({ "coinId": "bitcoin", "price": "60,000" }));
    assert_eq!(summary["updated"], 1);
    let coins = ok(&w, "crypto_prices", json!({}));
    assert_eq!(coins[0]["held"], "0.12345678");
    assert_eq!(coins[0]["unit_price"], "60000");
    let detail = ok(&w, "get_asset", json!({ "assetId": btc }));
    assert_eq!(detail["asset"]["current_display"], "7407.41 USD", "0.12345678 × 60,000");
    ok(&w, "keep_alive", json!({}));

    // --- a household spreadsheet ---------------------------------------------------------
    let household = "Item;Type;Purchase price;Date bought;Where;Serial #;Tags\n\
                     Grandfather clock;Heirloom;1.250,00;15/03/2021;Hall;GC-881;heirloom\n\
                     Omega Speedmaster;Watches;4.100,50;02/11/2019;Safe;OM-12;\n\
                     Broken row;;12,00;31/02/2020;;;\n";
    let inspected = ok(&w, "inspect_spreadsheet", json!({ "contents": household }));
    assert_eq!(inspected["delimiter"], ";");
    assert_eq!(
        inspected["mapping"],
        json!([
            "name",
            "type",
            "acquired_price",
            "acquired_date",
            "storage_location",
            "detail:serial_number",
            "tags"
        ])
    );
    assert_eq!(
        inspected["options"]["date_order"], "day_month_year",
        "15/03 can only be day-first"
    );
    assert_eq!(inspected["options"]["decimal_comma"], true);

    let request = |skip: Value, apply: bool| {
        json!({ "contents": household, "mapping": inspected["mapping"],
                "options": inspected["options"], "skip": skip, "apply": apply })
    };
    let before = ok(&w, "list_assets", json!({})).as_array().unwrap().len();
    let preview = ok(&w, "import_spreadsheet", request(json!([]), false));
    assert_eq!((preview["ready"].as_u64(), preview["errors"].as_u64()), (Some(2), Some(1)));
    assert_eq!(preview["rows"][1]["paid"], "4100.50 USD");
    assert_eq!(preview["rows"][1]["type_label"], "Watch");
    assert!(preview["rows"][0]["warnings"][0].as_str().unwrap().contains("Heirloom"));
    assert!(preview["rows"][2]["error"].as_str().unwrap().contains("not a date"));
    assert_eq!(
        ok(&w, "list_assets", json!({})).as_array().unwrap().len(),
        before,
        "a preview writes nothing"
    );
    let refused = ok(&w, "import_spreadsheet", request(json!([]), true));
    assert_eq!(refused["applied"], false, "not while a row has an error");

    let applied = ok(&w, "import_spreadsheet", request(json!([2]), true));
    assert_eq!(applied["applied"], true);
    assert_eq!(ok(&w, "list_assets", json!({})).as_array().unwrap().len(), before + 2);
    let again = ok(&w, "import_spreadsheet", request(json!([2]), false));
    assert!(again["rows"][1]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().contains("already in the catalog")));
    assert_eq!(
        ok(&w, "inspect_spreadsheet", json!({ "contents": household }))["remembered"],
        true
    );
    for a in ok(&w, "list_assets", json!({})).as_array().unwrap() {
        if a["name"] == "Grandfather clock" || a["name"] == "Omega Speedmaster" {
            ok(&w, "delete_asset", json!({ "assetId": a["asset_id"] }));
            ok(&w, "purge_trash", json!({ "assetId": a["asset_id"] }));
        }
    }

    // --- photos -----------------------------------------------------------------------
    let photo_path = dir.path().join("slab.png");
    image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([x as u8 * 3, y as u8 * 4, 90]))
        .save(&photo_path)
        .unwrap();
    let first =
        ok(&w, "import_photo", json!({ "assetId": btc, "path": photo_path.to_str().unwrap() }));
    assert_eq!(first["media_type"], "image/png");
    // A document is described, kept out of the gallery, and found by title.
    let pdf_path = dir.path().join("Purchase receipt.pdf");
    std::fs::write(&pdf_path, b"%PDF-1.4 receipt").unwrap();
    let receipt = ok(
        &w,
        "import_photo",
        json!({ "assetId": btc, "path": pdf_path.to_str().unwrap(),
                "details": { "kind": "receipt", "date": "2024-01-02" } }),
    );
    // The viewer reads it as bytes over IPC, from this asset only.
    let read = json!({ "assetId": btc, "objectId": receipt["object_id"] });
    assert_eq!(raw(&w, "read_attachment", read), b"%PDF-1.4 receipt");
    assert!(invoke(
        &w,
        "read_attachment",
        json!({ "assetId": eagles, "objectId": receipt["object_id"] })
    )
    .is_err());
    let attached = ok(&w, "list_photos", json!({ "assetId": btc }));
    let doc =
        attached.as_array().unwrap().iter().find(|p| p["object_id"] == receipt["object_id"]);
    assert_eq!(doc.unwrap()["title"], "Purchase receipt", "titled from the file name");
    assert_eq!(ok(&w, "search_assets", json!({ "query": "purchase receipt" })), json!([btc]));
    ok(
        &w,
        "describe_attachment",
        json!({ "assetId": btc, "objectId": receipt["object_id"],
                "details": { "kind": "appraisal", "title": "Appraisal", "note": "" } }),
    );
    ok(&w, "remove_photo", json!({ "assetId": btc, "objectId": receipt["object_id"] }));

    // An attachment can be taken back out, decrypted — but only from the
    // asset it belongs to.
    let copy = dir.path().join("copy.png");
    ok(
        &w,
        "export_attachment",
        json!({ "assetId": btc, "objectId": first["object_id"], "path": copy.to_str().unwrap() }),
    );
    assert_eq!(std::fs::read(&copy).unwrap(), std::fs::read(&photo_path).unwrap());
    assert!(invoke(
        &w,
        "export_attachment",
        json!({ "assetId": eagles, "objectId": first["object_id"],
                "path": dir.path().join("other.png").to_str().unwrap() }),
    )
    .is_err());
    let again =
        ok(&w, "import_photo", json!({ "assetId": btc, "path": photo_path.to_str().unwrap() }));
    assert_eq!(again["deduplicated"], true);
    let photos = ok(&w, "list_photos", json!({ "assetId": btc }));
    assert_eq!(photos.as_array().unwrap().len(), 1, "the same file twice is one photo");
    ok(&w, "set_primary_photo", json!({ "assetId": btc, "objectId": first["object_id"] }));
    let row = ok(&w, "list_assets", json!({}));
    let row = row.as_array().unwrap().iter().find(|a| a["asset_id"] == btc).unwrap().clone();
    assert_eq!(row["primary_photo"], first["object_id"]);
    ok(&w, "remove_photo", json!({ "assetId": btc, "objectId": first["object_id"] }));
    assert_eq!(ok(&w, "list_photos", json!({ "assetId": btc })), json!([]));
    // Deleting moves to the trash: out of the catalog, restorable, and only
    // then removable for good.
    ok(&w, "delete_asset", json!({ "assetId": btc }));
    let trash = ok(&w, "list_trash", json!({}));
    assert_eq!(trash.as_array().unwrap().len(), 1);
    assert_eq!(trash[0]["asset_id"], btc);
    assert!(trash[0]["purge_on"].as_str().is_some());
    assert!(!ok(&w, "list_assets", json!({}))
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["asset_id"] == btc));
    ok(&w, "restore_asset", json!({ "assetId": btc }));
    assert!(invoke(&w, "purge_trash", json!({ "assetId": btc })).is_err(), "not trashed");
    ok(&w, "delete_asset", json!({ "assetId": btc }));
    assert_eq!(ok(&w, "purge_trash", json!({ "assetId": btc })), 1);
    assert_eq!(ok(&w, "list_trash", json!({})), json!([]));

    // --- credentials, backup and restore -------------------------------------------
    let wrong = invoke(
        &w,
        "change_passphrase",
        json!({ "current": "nope", "newPassphrase": "another long passphrase" }),
    )
    .unwrap_err();
    assert!(wrong["message"].as_str().unwrap().contains("not correct"));
    ok(
        &w,
        "change_passphrase",
        json!({ "current": PASS, "newPassphrase": "another long passphrase" }),
    );

    let backups = dir.path().join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    let backup = ok(&w, "backup_vault", json!({ "directory": backups.to_str().unwrap() }));
    assert!(
        ok(&w, "vault_status", json!({}))["unlocked"].as_bool().unwrap(),
        "backup keeps the session"
    );
    assert_eq!(ok(&w, "get_settings", json!({}))["last_backup_at"], backup["created_at"]);

    // The backup centre remembers it, and can prove it restores.
    let centre = ok(&w, "backup_centre", json!({}));
    assert_eq!(centre["history"][0]["path"], backup["path"]);
    assert_eq!(centre["folder"], backups.to_str().unwrap(), "the folder is remembered");
    assert_eq!(centre["reminder_days"], 30);
    let failed = invoke(
        &w,
        "verify_backup",
        json!({ "directory": backup["path"], "secret": "not it", "useRecoveryKey": false }),
    );
    assert!(failed.is_err());
    let centre = ok(&w, "backup_centre", json!({}));
    assert_eq!(centre["history"][0]["verified_ok"], false, "a failed check is recorded too");
    let verified = ok(
        &w,
        "verify_backup",
        json!({ "directory": backup["path"], "secret": "another long passphrase",
                "useRecoveryKey": false }),
    );
    assert_eq!(verified["this_vault"], true);
    assert_eq!(verified["objects"], verified["decrypted"]);
    assert_eq!(ok(&w, "backup_centre", json!({}))["last_verified_at"], verified["verified_at"]);
    // Backing up again goes to the remembered folder.
    ok(
        &w,
        "update_backup_preferences",
        json!({ "preferences": { "folder": backups.to_str().unwrap(), "keep": 0, "reminder_days": 14 } }),
    );
    assert_eq!(ok(&w, "get_settings", json!({}))["backup_reminder_days"], 14);

    // Delete an asset after the backup; restoring must bring it back.
    ok(&w, "delete_asset", json!({ "assetId": comic }));
    assert_eq!(ok(&w, "list_assets", json!({})).as_array().unwrap().len(), 1);

    // The backup was made after the passphrase change, so the old one does
    // not open it — and a backup that cannot be opened replaces nothing.
    let refused = invoke(
        &w,
        "restore_vault",
        json!({ "directory": backup["path"], "secret": PASS, "useRecoveryKey": false }),
    )
    .unwrap_err();
    assert_eq!(refused["kind"], "cannot_unlock");
    assert_eq!(ok(&w, "vault_status", json!({}))["unlocked"], true, "session untouched");
    assert_eq!(ok(&w, "list_assets", json!({})).as_array().unwrap().len(), 1);

    let restored = ok(
        &w,
        "restore_vault",
        json!({ "directory": backup["path"], "secret": recovery, "useRecoveryKey": true }),
    );
    assert_eq!(restored["objects"], 0);
    assert_eq!(restored["opened"], true);
    assert_eq!(
        ok(&w, "vault_status", json!({}))["unlocked"],
        true,
        "a verified restore opens the restored vault"
    );
    assert_eq!(
        ok(&w, "list_assets", json!({})).as_array().unwrap().len(),
        2,
        "the deleted asset is back"
    );

    let rotated =
        ok(&w, "rotate_recovery_key", json!({ "passphrase": "another long passphrase" }));
    assert_ne!(rotated["recovery_key"].as_str().unwrap(), recovery);

    ok(&w, "lock_vault", json!({}));
    let stale =
        invoke(&w, "unlock_vault", json!({ "secret": recovery, "useRecoveryKey": true }))
            .unwrap_err();
    assert_eq!(stale["kind"], "cannot_unlock", "a rotated-out recovery key stops working");
}
