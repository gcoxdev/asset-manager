# Releasing

What has to be true before a build is offered to anyone, and how someone
who downloads it can check it is the build that was made.

## Before tagging

- [ ] `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`,
      `npm --prefix apps/desktop test` and the frontend build pass on the
      pinned toolchain (`rust-toolchain.toml`) — CI runs exactly these.
- [ ] `cargo audit` and `npm --prefix apps/desktop audit --omit=dev` are
      clean, or each finding is assessed and noted in the release notes.
- [ ] New dependencies since the last release are reviewed: licence
      compatible with AGPL-3.0, maintained, and needed.
- [ ] Vendored crypto (`crates/am-crypto/VENDOR.md`) is checked against
      upstream QiRing for security fixes since the recorded commit.
- [ ] A schema migration, if any, is tested from every earlier released
      schema (see `migrate.rs` tests) and documented in
      `docs/vault-format.md`.
- [ ] Placeholder icons are replaced (threat model §7).

## Clean-machine checks, per platform

On a machine that has never run Asset Manager — a fresh VM is fine — with
the built artifact, not a development build:

1. Install. Create a vault; complete the recovery ceremony.
2. Add an asset with a photo and a PDF receipt. Back up to a folder.
3. **Verify** the backup from Settings → Backups.
4. Copy the backup to a second clean machine. Restore it there **with the
   recovery key only** (no passphrase, no OS keyring). Check the photo and
   the receipt open.
5. Lock, then unlock with the passphrase; let the vault auto-lock while an
   edit dialog and the recovery screen are open, and confirm nothing
   remains on screen.

Do not claim a platform is supported until it has passed these.

## Artifacts and checksums

`npm run build:<target>` writes each artifact with a `.sha256` file beside
it. Publish both. Then sign the checksums so a mirror cannot substitute
both file and checksum:

- **All platforms:** collect the `.sha256` lines into `SHA256SUMS` and sign
  it with the project's release key: `gpg --armor --detach-sign SHA256SUMS`.
  Publish `SHA256SUMS` and `SHA256SUMS.asc`; publish the key's fingerprint
  in the README and somewhere independent of the release page.
- **Windows:** sign the `.msi` with an Authenticode certificate.
- **macOS:** sign with a Developer ID certificate and notarize the `.dmg`.
- **Linux:** the signed `SHA256SUMS` covers the `.AppImage` and `.deb`.

Users check a download with `sha256sum -c AssetManager.AppImage.sha256`
(after verifying `SHA256SUMS.asc` with `gpg --verify`).

## What not to claim

- **Reproducible builds:** not until independent builds have been compared
  byte for byte. Until then, the checksums say "this is the file that was
  published", not "this file is what the source produces".
- **Automatic updates:** there are none. Any future updater must verify a
  signature before installing and must not downgrade silently.
