# Asset Manager

A local-first desktop app for cataloging and valuing physical and digital
assets — precious metals, comics, trading cards, coins, crypto, and other
collectibles.

> **Status: pre-alpha.** Foundations only. There is no application to run yet.

## Why

Two jobs, in priority order:

1. **Catalog** — record what you own: photos, condition, provenance,
   acquisition cost, and storage location.
2. **Value** — track what it's worth, with as much automation as each asset
   class honestly allows.

Metals and crypto have real price feeds. Collectibles mostly do not, so manual
valuation is a first-class path rather than a fallback.

## Security

The catalog is an itemized list of valuables **including where they are
stored**. It is encrypted at rest, by default, with no opt-in step:

- **Argon2id** derives a key-encryption key from your passphrase, which wraps
  a random data key. A recovery key wraps the same data key, so recovery
  needs no backdoor.
- **XChaCha20-Poly1305** protects photos, in a chunked streaming format that
  rejects truncation, reordering and tampering.
- **HKDF-SHA256** derives separate subkeys for database, objects and
  thumbnails, so no key is reused across purposes.
- Photos on disk are ciphertext under opaque random filenames. Copying the
  vault directory without the credentials yields nothing.

**There is no password reset.** Lose both the passphrase and the recovery key
and the catalog is unrecoverable — by design, since there is no account and no
server. Keep the printed recovery sheet.

What this does *not* protect against: a compromised machine while the vault is
unlocked, or anything you deliberately export (CSV, PDF reports). A threat
model documenting this properly is part of the first release.

## Building

Requires Rust 1.93.1 (pinned in `rust-toolchain.toml`).

```bash
cargo test --workspace
```

Primary distribution target is a Linux AppImage; macOS and Windows bundles
follow the same Tauri build matrix.

## Layout

| Path | Purpose |
|---|---|
| `crates/am-crypto` | Key hierarchy and encrypted object format |

More crates land as the storage and domain layers are built.

## License

[AGPL-3.0](LICENSE). Parts of `am-crypto` are adapted from
[QiRing](https://github.com/gcoxdev/QiRing); see
[`crates/am-crypto/VENDOR.md`](crates/am-crypto/VENDOR.md).
