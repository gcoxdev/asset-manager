//! Records real command responses for the browser tests to replay.
//!
//! The browser tests (`npm run test:ui`) drive the actual frontend against a
//! stand-in for the backend. Hand-written stand-in data would drift from
//! what the commands really return; this builds a small vault through the
//! real command table and saves every response the tests need, so the UI is
//! always tested against the current shapes.
//!
//! Writes only when `UI_FIXTURES_DIR` is set; otherwise it is a no-op, and
//! `cargo test` stays free of side effects.

mod common;

use std::collections::BTreeMap;

use common::{sample_pdf, Recorder};
use serde_json::json;

#[test]
fn record_ui_fixtures() {
    let Some(out) = std::env::var_os("UI_FIXTURES_DIR") else {
        eprintln!("UI_FIXTURES_DIR not set; nothing recorded");
        return;
    };
    std::fs::create_dir_all(&out).unwrap();
    let out = std::fs::canonicalize(&out).unwrap();
    let out = out.as_path();

    let dir = tempfile::tempdir().unwrap();
    let (_app, w) = common::app(&dir.path().join("vault"));
    let mut r = Recorder { w: &w, calls: BTreeMap::new() };

    // --- the sample vault ---------------------------------------------------
    r.run("create_vault", json!({ "passphrase": "correct horse battery staple" }));
    r.run("confirm_recovery_saved", json!({}));
    let eagles = r.run(
        "create_asset",
        json!({ "form": {
            "type_id": "sovereign_coin", "name": "Gold Eagles", "quantity": "10", "quantity_unit": "coin",
            "acquired_date": "2024-01-02", "acquired_price": "20000", "storage_location": "Safe",
            "notes": "", "currency": "USD", "pricing": "market",
            "attrs": { "metal": "XAU", "weight_per_item": "1.0909", "weight_unit": "troy_oz",
                       "weight_basis": "gross", "purity": "0.9167", "preset": "age" }
        }}),
    );
    r.run("set_spot_price", json!({ "metal": "XAU", "price": "2000", "currency": "USD" }));
    let comic = r.run(
        "create_asset",
        json!({ "form": {
            "type_id": "comic", "name": "Amazing Fantasy #15", "quantity": "1", "quantity_unit": "item",
            "acquired_date": "2010-05-01", "acquired_price": "1100", "notes": "", "currency": "USD",
            "attrs": { "title": "Amazing Fantasy", "issue": "15", "grader": "cgc", "grade": "9.8" }
        }}),
    );
    r.run(
        "set_prices",
        json!({ "entries": [{
            "asset_id": comic, "amount": "1250000", "currency": "USD", "asof": null,
            "basis": "estimated_resale", "provenance": "manual", "note": null,
            "evidence": { "comparables": [
                { "description": "CGC 9.8, Heritage", "price": "1200000", "kind": "sold", "date": "2026-06-01" }],
              "low": "1100000", "high": "1400000", "confidence": "medium" }
        }]}),
    );
    let pdf_path = out.join("sample.pdf");
    std::fs::write(&pdf_path, sample_pdf()).unwrap();
    let pdf = r.run(
        "import_photo",
        json!({ "assetId": comic, "path": pdf_path.to_str().unwrap(),
                "details": { "kind": "appraisal", "title": "Appraisal 2026", "date": "2026-06-02", "note": "" } }),
    );
    let set = r.run("save_set", json!({ "name": "Gold", "targetCount": 20, "add": [eagles] }));

    // --- what the screens ask for --------------------------------------------
    r.record("vault_status", json!({}));
    r.record("session_state", json!({}));
    for cmd in [
        "get_settings",
        "list_assets",
        "asset_types",
        "collectible_types",
        "graders",
        "bullion_presets",
        "common_coins",
        "list_custom_types",
        "dashboard",
        "away_list",
        "list_tags",
        "list_locations",
        "list_saved_views",
        "list_sets",
        "get_preferences",
        "vault_info",
        "backup_centre",
        "list_trash",
        "spot_prices",
        "metals_provider_status",
        "crypto_prices",
        "crypto_provider_status",
        "list_rates",
    ] {
        r.record(cmd, json!({}));
    }
    r.record("care_due", json!({ "withinDays": 60 }));
    r.record("portfolio_series", json!({ "from": null, "maxPoints": 160 }));
    for id in [&eagles, &comic] {
        for cmd in ["get_asset", "list_photos", "list_care", "list_custody", "asset_revisions"]
        {
            r.record(cmd, json!({ "assetId": id }));
        }
    }
    r.record("set_members", json!({ "setId": set }));
    // The edit form asks the backend to name the item as it is typed.
    r.record(
        "validate_asset",
        json!({ "form": {
            "type_id": "comic", "name": null, "quantity_unit": "item", "currency": "USD",
            "attrs": { "title": "Amazing Fantasy", "issue": "15", "grader": "cgc", "grade": "9.8" }
        }}),
    );
    r.record("insurance_report", json!({ "options": { "include_documents": true } }));

    common::save(
        &out.join("fixtures.json"),
        json!({ "eagles": eagles, "comic": comic, "pdf": pdf["object_id"], "set": set }),
        &r.calls,
    );
}
