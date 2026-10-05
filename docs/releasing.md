# Releasing

What has to be true before a build is offered to anyone, and how someone
who downloads it can check it is the build that was made.

## Before tagging

- [ ] `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`,
      `npm --prefix apps/desktop test`, the browser tests
      (`npm --prefix apps/desktop run test:ui`) and the frontend build pass on
      the pinned toolchain (`rust-toolchain.toml`) — CI runs exactly these.
- [ ] The version in `Cargo.toml` and `apps/desktop/src-tauri/tauri.conf.json`
      is the one being tagged (the release workflow refuses a mismatch).
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

## Building, signing and publishing

Push a tag `vX.Y.Z`. The release workflow (`.github/workflows/release.yml`)
builds Linux (`.AppImage`, `.deb`), Windows (`.msi`) and macOS (universal
`.dmg`), and opens a **draft** release. Nothing is published until someone
checks the draft and publishes it by hand.

Every draft carries:

- **`SHA256SUMS`**: one line per file.
- **`SHA256SUMS.asc`**: a detached signature by the project's release key,
  so a mirror cannot substitute both a file and its checksum.
- **Build-provenance attestations**: a Sigstore-signed statement, in a
  public transparency log, that each file was built by this workflow from
  the tagged commit. These need no key of ours and are always produced.

Signing turns on with repository secrets (Settings → Secrets and variables
→ Actions). Each is optional; without it that step is skipped and the draft
says so.

| Secrets | What they sign |
|---|---|
| `RELEASE_GPG_KEY` (armored private key), `RELEASE_GPG_PASSPHRASE` | `SHA256SUMS` → `SHA256SUMS.asc` |
| `WINDOWS_CERTIFICATE` (`.pfx`, base64), `WINDOWS_CERTIFICATE_PASSWORD` | Authenticode on the app and the `.msi`, timestamped |
| `APPLE_CERTIFICATE` (`.p12`, base64), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` | Developer ID signature on the app and `.dmg` |
| `APPLE_ID`, `APPLE_PASSWORD` (app-specific), `APPLE_TEAM_ID` | Notarization of the `.dmg` |

Before the first signed release:

- [ ] Create the release key on an offline machine; keep the primary key
      offline and give the workflow a signing subkey only.
- [ ] Publish the key's fingerprint in the README **and** somewhere
      independent of the release page (a personal site, a keyserver), so a
      compromised release page cannot swap both key and signature.

A local build (`npm run build:<target>`) writes each artifact with a
`.sha256` file beside it; the workflow merges these into `SHA256SUMS`.

## Checking a download

```bash
gpg --verify SHA256SUMS.asc SHA256SUMS          # the key's fingerprint as published
sha256sum --check --ignore-missing SHA256SUMS   # the files you downloaded
gh attestation verify AssetManager.AppImage --repo gcoxdev/asset-manager
```

On Windows, the `.msi`'s Properties → Digital Signatures tab names the
signer; on macOS, `spctl --assess --type install -v AssetManager.dmg`
reports the Developer ID and notarization.

## What not to claim

- **Reproducible builds:** not until independent builds have been compared
  byte for byte. Until then, the checksums say "this is the file that was
  published", not "this file is what the source produces".
- **Automatic updates:** there are none. Any future updater must verify a
  signature before installing and must not downgrade silently.
