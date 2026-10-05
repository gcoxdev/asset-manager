//! Shared by the recorders: running commands on the mock runtime and keeping
//! their responses, and a sample PDF.

#![allow(dead_code)]

use std::collections::BTreeMap;

use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::WebviewWindow;

pub type Webview = WebviewWindow<tauri::test::MockRuntime>;

pub struct Recorder<'a> {
    pub w: &'a Webview,
    pub calls: BTreeMap<String, Vec<Value>>,
}

impl Recorder<'_> {
    /// Run a command; panic on failure.
    pub fn run(&self, cmd: &str, args: Value) -> Value {
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
    pub fn record(&mut self, cmd: &str, args: Value) -> Value {
        let result = self.run(cmd, args.clone());
        self.calls
            .entry(cmd.to_string())
            .or_default()
            .push(json!({ "args": args, "result": result }));
        result
    }
}

/// A three-page PDF in the standard Helvetica font, built with a correct
/// cross-reference table so readers need not repair it.
pub fn sample_pdf() -> Vec<u8> {
    let page = |text: &str| {
        let content = format!("BT /F1 28 Tf 72 700 Td ({text}) Tj ET");
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len())
    };
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R 8 0 R] /Count 3 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R /Resources << /Font << /F1 7 0 R >> >> >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R /Resources << /Font << /F1 7 0 R >> >> >>".to_string(),
        page("Appraisal - page one"),
        page("Appraisal - page two"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 9 0 R /Resources << /Font << /F1 7 0 R >> >> >>".to_string(),
        page("Appraisal - page three"),
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

/// The app, built on the mock runtime, with its one window. The vault goes
/// wherever `AM_VAULT_DIR` points (debug builds only).
pub fn app(vault: &std::path::Path) -> (tauri::App<tauri::test::MockRuntime>, Webview) {
    std::env::set_var("AM_VAULT_DIR", vault);
    let app = asset_manager_desktop_lib::configure(mock_builder())
        .build(mock_context(noop_assets()))
        .expect("app builds");
    let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    (app, w)
}

/// Write the recorded calls, with any IDs the tests need, as JSON.
pub fn save(path: &std::path::Path, ids: Value, calls: &BTreeMap<String, Vec<Value>>) {
    let fixtures = json!({ "ids": ids, "calls": calls });
    std::fs::write(path, serde_json::to_string_pretty(&fixtures).unwrap()).unwrap();
}
