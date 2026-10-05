//! Vaults made by earlier builds must keep opening.
//!
//! `tests/fixtures/vault-v17` is a real vault written by the build of
//! 2026-10-05 (schema 17; argon2 0.5, sha2 0.10, hkdf 0.12, rusqlite 0.37).
//! Every later build has to unlock it with the passphrase and with the
//! recovery key, read its records and decrypt its file. A dependency
//! upgrade that changes key derivation, hashing or the database format
//! fails here — before it ships and every existing vault stops opening.
//!
//! Never regenerate the fixture to make this pass: that would only prove the
//! new build reads its own output. Add a newer fixture beside it instead
//! (`AM_WRITE_COMPAT_FIXTURE=<dir> cargo test -p am-storage --test compat`).

use std::fs;
use std::path::Path;

use am_crypto::KdfParams;
use am_storage::header::Credential;
use am_storage::vault::{Vault, LOCK_FILE};

const PASS: &str = "compat fixture passphrase";
const NOW: &str = "2026-10-05T00:00:00Z";
const NAME: &str = "Compat fixture — 1 oz Gold Eagle";
const FILE: &[u8] =
    b"%PDF-1.4\n% compat fixture document, kept so its bytes can be checked\n%%EOF\n";

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else if entry.file_name() != LOCK_FILE {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// What the fixture holds besides the vault: the recovery key and the
/// stored file's ID. Test data only — this vault protects nothing.
fn secrets(dir: &Path) -> (String, String) {
    let text = fs::read_to_string(dir.join("fixture-secrets.txt")).unwrap();
    let get = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("{key} missing"))
            .to_string()
    };
    (get("recovery_key"), get("object_id"))
}

fn check(fixture: &Path) {
    let (recovery, object_id) = secrets(fixture);
    for (credential, secret) in
        [(Credential::Passphrase, PASS), (Credential::RecoveryKey, recovery.as_str())]
    {
        // A copy: opening migrates the schema forward and writes to it.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        copy_dir(&fixture.join("vault"), &root);

        let vault = Vault::unlock(&root, credential, secret)
            .unwrap_or_else(|e| panic!("{credential:?} no longer opens the v17 vault: {e}"));
        let all = am_storage::assets::list(&vault).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, NAME);
        assert_eq!(all[0].current_amount_minor, Some(345_000));
        let bytes = am_storage::objects::load_object(&vault, &root, &object_id).unwrap();
        assert_eq!(bytes.as_slice(), FILE, "the stored file decrypts to what was stored");
    }
}

#[test]
fn a_vault_from_schema_17_still_opens() {
    check(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault-v17"));
}

/// Writes a new fixture when `AM_WRITE_COMPAT_FIXTURE` names an empty or
/// missing directory; otherwise does nothing.
#[test]
fn write_fixture_when_asked() {
    let Some(out) = std::env::var_os("AM_WRITE_COMPAT_FIXTURE") else { return };
    let out = Path::new(&out);
    assert!(!out.join("vault").exists(), "refusing to overwrite an existing fixture");
    let root = out.join("vault");
    let fast = KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 };
    let (vault, recovery) = Vault::create(&root, PASS, &fast, NOW).unwrap();

    let id = am_storage::assets::create(
        &vault,
        &am_storage::assets::NewAsset {
            type_id: "generic".into(),
            name: NAME.into(),
            quantity: am_core::Decimal::ONE,
            quantity_unit: "item".into(),
            acquired_date: Some("2024-01-02".into()),
            effective_date: None,
            acquired_cost: None,
            acquired_from: None,
            storage_location: Some("Safe".into()),
            notes: "Written to prove later builds can still read it.".into(),
            insured: None,
            attrs: Default::default(),
            pricing: am_storage::assets::Pricing::Manual,
            review_every_days: None,
        },
        NOW,
    )
    .unwrap();
    am_storage::valuations::record_valuation(
        &vault,
        &am_storage::valuations::NewValuation {
            asset_id: id,
            quote_id: None,
            value: am_core::Money::new(345_000, am_core::Currency::new("USD").unwrap()),
            quantity_at_time: am_core::Decimal::ONE,
            basis: am_storage::valuations::Basis::EstimatedResale,
            provenance: am_storage::valuations::Provenance::Manual,
            inputs: serde_json::json!({}),
            asof: "2026-10-01".into(),
        },
        NOW,
    )
    .unwrap();
    let stored = am_storage::objects::import_object(&vault, &root, FILE, NOW).unwrap();
    drop(vault);
    let _ = fs::remove_file(root.join(LOCK_FILE));
    fs::write(
        out.join("fixture-secrets.txt"),
        format!("recovery_key={recovery}\nobject_id={}\n", stored.object_id),
    )
    .unwrap();
    check(out);
}
