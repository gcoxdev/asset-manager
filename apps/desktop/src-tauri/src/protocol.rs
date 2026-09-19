//! The `asset://` protocol handler.
//!
//! **This is a separate authorization surface from Tauri commands.** Tauri's
//! command permissions do not apply here: the WebView reaches this handler
//! directly, so every check the commands make must be made again, explicitly.
//!
//! Requests look like:
//!
//! ```text
//! asset://localhost/media/<object_id>            the original
//! asset://localhost/media/<object_id>?variant=256  a thumbnail
//! ```
//!
//! Why a protocol rather than base64 over IPC: base64 inflates payloads ~33%
//! and blocks the IPC channel, which a grid of photos would make obvious.
//!
//! Responses carry `Cache-Control: no-store` so the WebView does not write
//! decrypted images into its own disk cache — that would put plaintext copies
//! outside the vault, defeating the encryption.

use am_storage::objects::load_object;
use am_storage::thumbs::load_variant;
use tauri::http::{Request, Response, StatusCode};
use tauri::{Manager, UriSchemeContext};

use crate::paths::vault_root;
use crate::session::Session;

/// Object IDs are 32 lowercase hex characters. Anything else is rejected
/// before it reaches the filesystem, so path traversal is impossible by
/// construction rather than by sanitization.
fn is_valid_object_id(candidate: &str) -> bool {
    candidate.len() == 32 && candidate.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Only variants we actually generate are servable.
fn variant_name(raw: Option<&str>) -> Option<&'static str> {
    match raw {
        None => None,
        Some("256") => Some("thumb:256"),
        Some("1024") => Some("thumb:1024"),
        Some(_) => Some("__invalid__"),
    }
}

fn error(status: StatusCode) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Cache-Control", "no-store")
        .body(Vec::new())
        .expect("static response builds")
}

pub fn handle<R: tauri::Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    // Read-only surface. A mutating verb here would be a bug, not a feature.
    if request.method() != "GET" {
        return error(StatusCode::METHOD_NOT_ALLOWED);
    }

    let uri = request.uri();
    let path = uri.path().trim_start_matches('/');
    let Some(object_id) = path.strip_prefix("media/") else {
        return error(StatusCode::NOT_FOUND);
    };

    if !is_valid_object_id(object_id) {
        return error(StatusCode::BAD_REQUEST);
    }

    let raw_variant = uri.query().and_then(|q| {
        q.split('&')
            .find_map(|pair| pair.strip_prefix("variant="))
            .map(|v| v.to_string())
    });
    let variant = variant_name(raw_variant.as_deref());
    if variant == Some("__invalid__") {
        return error(StatusCode::BAD_REQUEST);
    }

    let app = ctx.app_handle();
    let session = app.state::<Session>();

    // The lock check that matters: a locked vault serves nothing, regardless
    // of what the UI is displaying.
    let Ok(root) = vault_root(app) else {
        return error(StatusCode::INTERNAL_SERVER_ERROR);
    };

    let result = session.with_vault(|vault| {
        let bytes = match variant {
            Some(name) => match load_variant(vault, &root, object_id, name) {
                Ok(Some(b)) => Some((b, "image/jpeg".to_string())),
                // A variant that has not been generated yet is a miss, not an
                // error: the caller falls back to the original.
                Ok(None) => None,
                Err(_) => None,
            },
            None => None,
        };

        if let Some((bytes, media_type)) = bytes {
            return Ok(Some((bytes.to_vec(), media_type)));
        }

        // Original, or a variant that was not available.
        let media_type: String = vault
            .conn()
            .query_row(
                "SELECT media_type FROM objects WHERE object_id = ?1 AND gc_state = 'live'",
                [object_id],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "application/octet-stream".to_string());

        match load_object(vault, &root, object_id) {
            Ok(bytes) => Ok(Some((bytes.to_vec(), media_type))),
            Err(_) => Ok(None),
        }
    });

    match result {
        // Locked, or any session failure: reveal nothing about why.
        Err(_) => error(StatusCode::FORBIDDEN),
        Ok(None) => error(StatusCode::NOT_FOUND),
        Ok(Some((bytes, media_type))) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", media_type)
            // Keeps decrypted images out of the WebView's own disk cache.
            .header("Cache-Control", "no-store")
            // The bytes are images; make sure nothing tries to run them.
            .header("X-Content-Type-Options", "nosniff")
            .header("Content-Security-Policy", "default-src 'none'; sandbox")
            .body(bytes)
            .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_ids_must_be_exactly_32_hex_chars() {
        assert!(is_valid_object_id("0123456789abcdef0123456789abcdef"));
        assert!(is_valid_object_id("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"));

        assert!(!is_valid_object_id(""), "empty");
        assert!(!is_valid_object_id("0123456789abcdef"), "too short");
        assert!(!is_valid_object_id("0123456789abcdef0123456789abcdef0"), "too long");
        assert!(!is_valid_object_id("0123456789abcdef0123456789abcdeg"), "non-hex");
    }

    #[test]
    fn path_traversal_attempts_are_rejected_by_the_id_check() {
        for attempt in [
            "../../etc/passwd",
            "../../../vault.header",
            "..%2F..%2Fvault.header",
            "0123456789abcdef/../../../etc/passwd",
            "/etc/passwd",
            "0123456789abcdef0123456789abcde/",
        ] {
            assert!(!is_valid_object_id(attempt), "accepted traversal attempt: {attempt}");
        }
    }

    #[test]
    fn only_known_variants_are_servable() {
        assert_eq!(variant_name(None), None);
        assert_eq!(variant_name(Some("256")), Some("thumb:256"));
        assert_eq!(variant_name(Some("1024")), Some("thumb:1024"));

        // Anything else is an explicit rejection, not a silent fallback —
        // otherwise a typo would quietly serve the full-size original.
        assert_eq!(variant_name(Some("512")), Some("__invalid__"));
        assert_eq!(variant_name(Some("../original")), Some("__invalid__"));
        assert_eq!(variant_name(Some("")), Some("__invalid__"));
    }
}
