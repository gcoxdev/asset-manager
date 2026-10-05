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

use std::collections::BTreeMap;

use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::WebviewWindow;

type Webview = WebviewWindow<tauri::test::MockRuntime>;

struct Recorder<'a> {
    w: &'a Webview,
    calls: BTreeMap<String, Vec<Value>>,
}

impl Recorder<'_> {
    /// Run a command; panic on failure.
    fn run(&self, cmd: &str, args: Value) -> Value {
        get_ipc_response(
            self.w,
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
        .unwrap_or_else(|e| panic!("{cmd} failed: {e:?}"))
    }

    /// Run a command and keep its response for replay.
    fn record(&mut self, cmd: &str, args: Value) -> Value {
        let result = self.run(cmd, args.clone());
        self.calls
            .entry(cmd.to_string())
            .or_default()
            .push(json!({ "args": args, "result": result }));
        result
    }
}

/// A two-page PDF in the standard Helvetica font, built with a correct
/// cross-reference table so readers need not repair it.
fn sample_pdf() -> Vec<u8> {
    let page = |text: &str| {
        let content = format!("BT /F1 28 Tf 72 700 Td ({text}) Tj ET");
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len())
    };
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R /Resources << /Font << /F1 7 0 R >> >> >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R /Resources << /Font << /F1 7 0 R >> >> >>".to_string(),
        page("Appraisal - page one"),
        page("Appraisal - page two"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .bytes(),
    );
    out
}

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
    // Debug builds honour this override; release builds never read it.
    std::env::set_var("AM_VAULT_DIR", dir.path().join("vault"));
    let app = asset_manager_desktop_lib::configure(mock_builder())
        .build(mock_context(noop_assets()))
        .expect("app builds");
    let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
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

    let fixtures = json!({
        "ids": { "eagles": eagles, "comic": comic, "pdf": pdf["object_id"], "set": set },
        "calls": r.calls,
    });
    std::fs::write(out.join("fixtures.json"), serde_json::to_string_pretty(&fixtures).unwrap())
        .unwrap();
}
