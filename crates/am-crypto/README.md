# Asset Manager vault cryptography

`am-crypto` is maintained as part of Asset Manager. Its source, tests, and
format contracts live in this repository. Crypto maintenance and release
decisions are handled here.

## Implementation

The crate uses library implementations of cryptographic primitives:

- `argon2` derives key-encryption keys from passphrases and recovery keys.
- `chacha20poly1305` encrypts wrapped data keys and file chunks using
  XChaCha20-Poly1305.
- `hkdf` and `sha2` derive separate database, object, and thumbnail keys.
- `rand` supplies OS randomness; `zeroize` supplies secret-buffer cleanup.

Asset Manager maintains the code that combines these primitives:

- `kdf.rs`: parameter bounds, key derivation, purpose labels, and base32
  recovery-key formatting.
- `wrap.rs`: authenticated wrapping of the vault data key for password and
  recovery access.
- `stream.rs`: chunked object encryption, including authenticated context,
  chunk ordering, and end-of-stream markers.

The vault header and database integration live in `am-storage`. See the
[vault format](../../docs/vault-format.md) and
[threat model](../../docs/threat-model.md) for their contracts and limits.

## Maintenance

- Review dependency advisories with `cargo audit` and review the crypto
  dependency updates proposed by this repository's Dependabot configuration.
- Review changes to key handling, derivation parameters, purpose labels,
  authenticated metadata, nonce handling, and object framing against the
  documented vault format.
- After crypto changes, run `cargo test -p am-crypto` and
  `cargo test -p am-storage`, including compatibility and leakage tests.
  Preserve coverage for wrong credentials, tampering, truncation, reordering,
  context substitution, and chunk boundaries.
- Treat changes to on-disk encodings or key derivation as compatibility
  changes. Preserve support for existing vaults or provide a documented,
  tested migration before release.

Passing tests and dependency audits do not establish an independent
cryptographic review of the application-specific format.
