//! Reading barcodes from photographs of graded slabs.
//!
//! # Why a photo rather than a live camera
//!
//! Live capture would need `getUserMedia`, a `media-src` CSP relaxation and
//! WebKitGTK camera plumbing — a permanent privacy surface for a feature used
//! occasionally. Decoding a photo reuses the import path that already exists,
//! adds no permission, and works with whatever camera the user already
//! trusts. A slab is a static object; there is nothing to gain from a live
//! viewfinder.
//!
//! # What this does and does not tell you
//!
//! A decoded barcode is a **claim printed on a label**, not proof of
//! anything. It identifies which certificate to look up; it does not verify
//! the item, the grade, or that the label belongs to what is inside the
//! holder. Callers must not present a scan as authentication.

use am_core::Grader;
use serde::Serialize;

use crate::objects::{load_object, ObjectError};
use crate::vault::Vault;

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("could not decode that image")]
    Decode,
    #[error("no barcode found in the image")]
    NotFound,
    #[error(transparent)]
    Object(#[from] ObjectError),
}

/// A barcode found in an image.
#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    /// Raw decoded text, exactly as printed.
    pub text: String,
    /// Barcode symbology, e.g. `CODE_128`, `QR_CODE`.
    pub format: String,
    /// Grader inferred from the payload's shape, when it is unambiguous.
    /// `None` means "we could not tell", never a guess.
    pub likely_grader: Option<String>,
    /// The digits that look like a certificate number.
    pub cert_number: Option<String>,
}

/// Decode the first barcode in an image.
pub fn scan_image(bytes: &[u8]) -> Result<ScanResult, ScanError> {
    // Decode the image first so "not an image" and "no barcode" stay
    // distinguishable; rxing collapses both into one error type.
    image::load_from_memory(bytes).map_err(|_| ScanError::Decode)?;

    // TryHarder trades speed for tolerance of the angles and glare a phone
    // photo of a glossy slab actually produces.
    let mut hints = rxing::DecodeHints { TryHarder: Some(true), ..Default::default() };

    let result = rxing::helpers::detect_in_buffer_with_hints(bytes, None, &mut hints)
        .map_err(|_| ScanError::NotFound)?;

    let text = result.getText().trim().to_string();
    let format = format!("{:?}", result.getBarcodeFormat());

    Ok(ScanResult {
        cert_number: extract_cert_number(&text),
        likely_grader: infer_grader(&text).map(|g| g.as_str().to_string()),
        text,
        format,
    })
}

/// Scan a photo already stored in the vault.
pub fn scan_stored_object(
    vault: &Vault,
    root: &std::path::Path,
    object_id: &str,
) -> Result<ScanResult, ScanError> {
    let plaintext = load_object(vault, root, object_id)?;
    scan_image(&plaintext)
}

/// Pull a certificate number out of a decoded payload.
///
/// Slab barcodes vary: some encode the bare number, others prefix it or
/// append a grade. The longest run of digits is the certificate number in
/// every format seen, and returning `None` is better than returning a wrong
/// number that someone then trusts.
fn extract_cert_number(text: &str) -> Option<String> {
    let mut best: Option<&str> = None;
    let mut start: Option<usize> = None;

    let bytes = text.as_bytes();
    for i in 0..=bytes.len() {
        let is_digit = i < bytes.len() && bytes[i].is_ascii_digit();
        match (is_digit, start) {
            (true, None) => start = Some(i),
            (false, Some(from)) => {
                let run = &text[from..i];
                if best.is_none_or(|b| run.len() > b.len()) {
                    best = Some(run);
                }
                start = None;
            }
            _ => {}
        }
    }

    // Certificate numbers run to roughly 7–10 digits. A shorter run is more
    // likely a year or a grade than a cert.
    best.filter(|run| run.len() >= 6).map(str::to_string)
}

/// Infer the grader from an explicit marker in the payload.
///
/// Only when the payload names one. Grader-by-digit-count heuristics exist
/// but collide across services, and a wrong grader attached to a cert number
/// sends someone to the wrong lookup with no sign anything went astray.
fn infer_grader(text: &str) -> Option<Grader> {
    let upper = text.to_ascii_uppercase();
    for grader in Grader::ALL {
        if *grader == Grader::Raw {
            continue;
        }
        let name = grader.as_str().to_ascii_uppercase();
        if upper.contains(&name) {
            return Some(*grader);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_bare_certificate_number() {
        assert_eq!(extract_cert_number("1234567890").as_deref(), Some("1234567890"));
    }

    #[test]
    fn extracts_the_longest_digit_run() {
        // A payload carrying a grade and a cert: the cert is the long one.
        assert_eq!(extract_cert_number("98 1234567").as_deref(), Some("1234567"));
        assert_eq!(extract_cert_number("PSA 09876543 10").as_deref(), Some("09876543"));
    }

    #[test]
    fn preserves_leading_zeros() {
        // A cert number is an identifier, not a quantity.
        assert_eq!(extract_cert_number("0012345678").as_deref(), Some("0012345678"));
    }

    #[test]
    fn rejects_runs_too_short_to_be_a_cert() {
        // Better to return nothing than a year someone then trusts as a cert.
        assert_eq!(extract_cert_number("1962"), None);
        assert_eq!(extract_cert_number("grade 9.8"), None);
        assert_eq!(extract_cert_number("no digits here"), None);
        assert_eq!(extract_cert_number(""), None);
    }

    #[test]
    fn infers_a_grader_only_when_the_payload_names_one() {
        assert_eq!(infer_grader("CGC 1234567"), Some(Grader::Cgc));
        assert_eq!(infer_grader("psa-0987654"), Some(Grader::Psa));
        assert_eq!(infer_grader("PCGS 12345678"), Some(Grader::Pcgs));

        // No marker: say nothing rather than guess. A wrong grader sends
        // someone to the wrong lookup with no sign of the error.
        assert_eq!(infer_grader("1234567890"), None);
        assert_eq!(infer_grader("unlabelled"), None);
    }

    #[test]
    fn ungraded_is_never_inferred() {
        // "Raw" is the absence of grading, so no payload should produce it.
        assert_eq!(infer_grader("RAW 1234567"), None);
    }

    #[test]
    fn a_non_image_is_a_decode_error_not_a_missing_barcode() {
        // The two failures mean different things to a user: "that is not a
        // photo" versus "no barcode in this photo".
        assert!(matches!(scan_image(b"not an image at all"), Err(ScanError::Decode)));
    }

    /// Round-trip against a genuine barcode.
    ///
    /// The other tests exercise the parsing helpers; this one proves the
    /// decode path itself works, which is the part that would otherwise only
    /// be verified by someone photographing a slab.
    #[test]
    fn decodes_a_real_code128_barcode() {
        use rxing::Writer;

        let payload = "CGC 0012345678";
        let matrix = rxing::oned::Code128Writer
            .encode(payload, &rxing::BarcodeFormat::CODE_128, 400, 120)
            .expect("encoding a test barcode");

        // BitMatrix -> PNG, as a photo of a label would arrive.
        let mut img = image::GrayImage::new(matrix.getWidth(), matrix.getHeight());
        for y in 0..matrix.getHeight() {
            for x in 0..matrix.getWidth() {
                let shade = if matrix.get(x, y) { 0u8 } else { 255u8 };
                img.put_pixel(x, y, image::Luma([shade]));
            }
        }
        let mut bytes = Vec::new();
        image::DynamicImage::ImageLuma8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();

        let result = scan_image(&bytes).expect("should decode its own barcode");
        assert_eq!(result.text, payload);
        assert_eq!(result.format, "CODE_128");
        assert_eq!(result.cert_number.as_deref(), Some("0012345678"));
        assert_eq!(result.likely_grader.as_deref(), Some("cgc"));
    }

    #[test]
    fn an_image_without_a_barcode_reports_not_found() {
        let blank = image::RgbImage::from_pixel(200, 200, image::Rgb([255, 255, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(blank)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();

        assert!(matches!(scan_image(&bytes), Err(ScanError::NotFound)));
    }
}
