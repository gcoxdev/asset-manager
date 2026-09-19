# Vendoring record — am-crypto

Parts of this crate are adapted from **QiRing**, by the same author.

| | |
|---|---|
| Upstream | https://github.com/gcoxdev/QiRing |
| License | AGPL-3.0 |
| Vendored from commit | `ad314ead6d2fc6d367b67ea28bcbccc8abeff387` |
| Upstream version | 0.1.2 |
| Vendored on | 2026-09-18 |
| Source path | `crates/qiring-crypto/src/lib.rs` |

Asset Manager is likewise AGPL-3.0, so the copyleft terms carry over without
a licensing conflict. Keep both projects on AGPL-3.0, or this vendoring has
to be revisited.

## What was taken

- Argon2id KEK derivation and its parameter bounds (8–256 MiB, 1–10
  iterations, 1–4 lanes), which stop a tampered header forcing absurd or
  trivially weak work
- The KEK-wraps-DEK key hierarchy, and the two-unlock-paths-to-one-DEK
  recovery design
- `Zeroizing` key handling and opaque error types that don't reveal which
  step failed

## What is new here (NOT from upstream)

QiRing encrypts small in-memory secrets. Asset Manager encrypts multi-megabyte
photos, so these are original and carry no upstream review:

- `stream.rs` — chunked streaming AEAD with an authenticated end-of-stream
  marker. Upstream has no streaming format.
- `kdf::derive_subkey` — HKDF-SHA256 domain separation across database /
  object / thumbnail purposes. **Upstream has no HKDF at all** (verified: no
  `hkdf`, `sha2`, or `blake3` anywhere in the QiRing workspace); it uses a
  single DEK.
- Base32 recovery keys with grouping and transcription normalization.
  Upstream uses unpadded URL-safe base64, which is denser but worse to read
  off a printed sheet.

## Pulling upstream fixes

QiRing runs Dependabot and `cargo audit`. Before each Asset Manager release,
check upstream for changes to `crates/qiring-crypto/` since the SHA above:

```bash
git -C ../qiring log --oneline ad314ea..HEAD -- crates/qiring-crypto/
```

Apply relevant fixes by hand — this is a fork, not a dependency — and update
the SHA above. Re-run the full `am-crypto` test suite afterwards; the
truncation, reordering and context-substitution tests are the ones that
matter most.
