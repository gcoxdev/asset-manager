# Vault format v1

Normative specification of Asset Manager's on-disk format. Where this document
and the code disagree, this document is the intent and the code is the bug —
except where a test asserts the behaviour, in which case the test wins and this
document needs updating.

Status: **implemented and tested** (`am-crypto`, `am-storage`). The format is
not yet frozen; it freezes at the first public release.

---

## 1. Layout

```
<app-data>/asset-manager/
├── vault.header          unencrypted, authenticated (§3)
├── catalog.db            SQLCipher database (§5)
├── catalog.db-wal        WAL; transient
├── objects/              encrypted originals (§4)
│   └── 3f/7a/3f7a9c02…   random 128-bit object ID, no extension
└── cache/
    └── thumbs/           encrypted derived variants
        └── 9b/21/9b21c7e4…
```

A **backup** is `vault.header` + `catalog.db` + `objects/`. `cache/` is
regenerable and excluded. **Omitting `vault.header` makes the backup
permanently undecryptable** — it holds the wrapped data key.

Filenames are opaque. Object IDs are random, carry no extension, and reveal
nothing about content, type or which thumbnail sizes exist. The plaintext hash
used for deduplication lives only inside the encrypted database.

---

## 2. Key hierarchy

```
passphrase ──Argon2id(salt_p, params_p)──> KEK_p ──wraps──┐
                                                          ├──> data key (32B random)
recovery key ─Argon2id(salt_r, params_r)──> KEK_r ──wraps──┘         │
                                                                     │ HKDF-SHA256
                          ┌──────────────────────┬───────────────────┤
                          ▼                      ▼                   ▼
                   db subkey              object subkey       thumbnail subkey
                   "am/v1/db"          "am/v1/object"       "am/v1/thumb"
```

- **Argon2id** defaults: 64 MiB, 3 iterations, parallelism 1. Accepted range
  8–256 MiB, 1–10 iterations, 1–4 lanes; a header outside those bounds is
  rejected **before** Argon2 runs, so a hostile header cannot demand gigabytes.
- **Two unlock paths, one data key.** Recovery therefore needs no backdoor, and
  changing the passphrase does not re-encrypt content.
- **HKDF-SHA256 expand** derives purpose subkeys, with a length-prefixed info
  string. The data key is already uniform, so no extract step is needed.
  Do **not** derive subkeys with Argon2 — 64 MiB per subkey buys nothing here.
- Derived keys are held in `Zeroizing` and wiped on drop.

### Recovery key

20 random bytes (160 bits), base32, grouped in fours with dashes:
`AB12-CD34-…`. Base32 rather than base64 because it is transcribed from a
printed sheet: case-insensitive, no visually ambiguous pairs. Normalization
strips all non-alphanumerics and uppercases, so dashes, spaces and lowercase
all parse.

A non-secret **fingerprint** (truncated SHA-256 of the normalized key, base32)
is stored in the header so a user can identify which printed sheet belongs to
which vault without revealing the key.

---

## 3. `vault.header`

Pretty-printed JSON. Unencrypted — it must be readable before any key exists.

```jsonc
{
  "format": "asset-manager-vault-v1",
  "format_version": 1,
  "vault_id": "<32 hex chars>",        // 128-bit random
  "key_epoch": 1,                      // increments on credential change
  "created_at": "2026-09-19T00:00:00Z",
  "passphrase_slot": { "salt": "<32 hex>", "params": {...}, "wrapped": {...} },
  "recovery_slot":   { "salt": "<32 hex>", "params": {...}, "wrapped": {...},
                       "wrapped_at_epoch": 1 },   // optional; see below
  "recovery_fingerprint": "<base32>"
}
```

### Authentication — there is no MAC

The header is authenticated by folding its fields into the **AAD of the wrapped
data key**. Tampering is detected because the unwrap fails. A standalone MAC is
impossible: the key to verify it with would derive from salts stored in this
same file.

AAD is canonical JSON of exactly these fields, in this order:

| Field | Note |
|---|---|
| `format` | |
| `format_version` | |
| `purpose` | `am/v1/wrap/passphrase` or `am/v1/wrap/recovery` |
| `vault_id` | |
| `key_epoch` | the slot's `wrapped_at_epoch` if present, else the header's `key_epoch` |
| `created_at` | |
| `slot_salt` | **only the slot being unwrapped** |
| `slot_params` | **only the slot being unwrapped** |

> **Any field not in this list is unprotected.** Adding a field to
> `VaultHeader` without adding it here leaves it silently editable. This is the
> single easiest way to weaken the format.

Only the *relevant* slot's salt and params are included, so rotating one
credential does not invalidate the other.

### Slot epochs

Changing a credential advances `key_epoch` but can re-wrap only that
credential's slot: the other slot's KEK derives from a secret the app does not
hold. So before the epoch advances, the untouched slot records the epoch it is
authenticated under as `wrapped_at_epoch`, and its AAD uses that value from
then on. A re-wrapped slot clears the field and follows the header again.

The field is absent until the first credential change, and absent means
"use the header's `key_epoch`" — exactly how every slot was authenticated
before the field existed, so older headers are unchanged. Its value is inside
the AAD, so editing it breaks the unwrap like any other tampering.

> **Repair of older vaults.** Builds before slot epochs advanced the epoch
> without pinning the other slot, which silently disabled it: a passphrase
> change killed the recovery key and vice versa. The wrapped key itself was
> intact. On unlock, if the normal unwrap fails and the slot has no
> `wrapped_at_epoch`, the KEK is derived once and the unwrap retried at each
> earlier epoch. A match must still pass the header↔database pairing check
> before it is accepted and pinned into the header. A wrong credential never
> matches; an edited epoch gains nothing, because pairing fails.

Field order is part of the format: reordering the `AuthenticatedHeader` struct
changes the AAD and breaks every existing vault.

### Two accepted consequences

1. **Tamper detection requires the correct credential.** A modified header is
   indistinguishable from a wrong passphrase until the right one is entered.
   Both therefore return the *same* error — distinguishing them would tell an
   attacker editing the header whether they had guessed the passphrase.
2. **Recovery is not revocation.** Re-wrapping the data key changes which
   passphrase opens *this* header. An **older backup** still holds a wrapper the
   old passphrase opens, and the data key is unchanged — so the old passphrase
   plus the old backup decrypts newer content too. Changing a passphrase is not
   key rotation; genuine rotation requires re-encrypting all content.

### Atomic updates

Write to a temporary file in the same directory, fsync, rename. The rename is
atomic; a crash leaves either the old or the new header, never a partial one.

---

## 4. Object format

Each encrypted object:

```
[24-byte base nonce] [chunk 0] [chunk 1] ... [chunk n]
```

- **64 KiB** plaintext per chunk; sealed chunks are 64 KiB + 16-byte tag. The
  last chunk may be shorter. An empty object still writes one final empty chunk,
  so "no chunks" is always malformed.
- **Base nonce is freshly random per seal** (192-bit, OS RNG). Per-chunk nonces
  XOR the little-endian chunk index into its last 8 bytes.

> **The nonce is never derived from the object ID.** Object IDs are random
> 128-bit values, so deriving a nonce from one gives no birthday-bound
> protection against two objects sharing a key+nonce pair — and
> XChaCha20-Poly1305 fails catastrophically under nonce reuse. A fresh random
> 192-bit nonce makes collision probability negligible and is re-randomized on
> every re-encryption.

### Per-chunk AAD

Length-prefixed encoding of:

```
vault_id ‖ object_id ‖ format_version ‖ purpose ‖ chunk_index ‖ is_final
```

- **Length-prefixed** so distinct contexts cannot collide by concatenation
  (`"ab"+"c"` vs `"a"+"bc"`).
- **`is_final` is determined by position in the stream, never by whether a
  chunk is short.** Inferring it from length corrupts objects whose size is an
  exact multiple of 64 KiB, and — far worse — makes whole-chunk truncation
  undetectable, because the new trailing chunk also reads as final. This was a
  real bug caught by the truncation test; do not reintroduce it.
- **No asset ID in the AAD.** One object may be attached to several assets, so
  binding to an asset would break deduplication and make moving an attachment
  invalidate the bytes. The association lives in the encrypted database.

### What this rejects

Truncation (byte- and chunk-level), reordering, tampering, wrong key, and
context substitution — an object moved between vaults, between object IDs,
between purposes, or across format versions.

### Deduplication

SHA-256 of the **plaintext**, stored only in the encrypted database as an index
to the object ID. Identical bytes are stored once regardless of how many assets
reference them.

Dedup is **byte-identical only**. Two separate photographs of the same item
produce different bytes and are not deduplicated; no design deduplicates them.

---

## 5. Database

SQLCipher (AES-256), opened with `bundled-sqlcipher-vendored-openssl` so no
system OpenSSL is required. Keyed with the `am/v1/db` subkey as raw bytes
(`PRAGMA key = "x'<hex>'"`), skipping SQLCipher's own KDF since the key is
already Argon2id-derived.

Required pragmas, in order:

| Pragma | Value | Why |
|---|---|---|
| `key` | raw hex | **must be first** — SQLCipher reads page 1 to derive the key |
| `temp_store` | `MEMORY` | SQLCipher does **not** encrypt file-based temp storage; a sort or FTS5 rebuild could otherwise leave plaintext outside the vault |
| `journal_mode` | `WAL` | concurrent reads during writes |
| `foreign_keys` | `ON` | |

Verified working: FTS5 (external-content, including `rebuild` and `optimize` at
5,000 rows under memory-only temp storage), JSON1, WAL.

### Header ↔ database pairing

The database stores `vault_id` and `key_epoch` in a metadata row, checked
against the header on every open.

Without this, restoring a backup's `vault.header` beside a current `catalog.db`
— or half-finishing a restore — produces an intact but permanently
undecryptable vault, and the failure *looks like a wrong passphrase*, sending
the user after the wrong problem. On mismatch, refuse to open and say so.

`key_epoch` increments on every credential change or rewrap.

### Schema versions

| Version | Change |
|---|---|
| 1 | Initial schema |
| 2 | `app_settings` (per-vault settings, including privacy opt-ins) |
| 3 | Effective dates normalized to calendar dates; type categories and the types the forms create; collectibles saved as `generic` reclassified from their fields; `pricing` (`manual`/`market`) and `review_every_days` on assets; FTS index extended to type-specific attributes |

---

## 6. Backup and restore

**SQLite's online backup API does not work on an encrypted database** —
SQLCipher returns *"backup is not supported with encrypted databases"*. This is
pinned by a test so a future release lifting the restriction surfaces as a
failure. Pause-and-copy is therefore the only protocol:

1. Pause writes; finish or cancel in-flight imports.
2. `PRAGMA wal_checkpoint(TRUNCATE)` — without this a copy can miss committed
   transactions still living in the WAL.
3. Close all database connections.
4. Copy `vault.header`, `catalog.db`, and `objects/`. Pin referenced objects
   against GC until the copy completes.
5. Write a manifest: format version, vault ID, key epoch, object list, sizes
   and hashes.

The manifest carries an object list **from v1**, even though v1 copies
everything. Objects are immutable and content-addressed, so incremental backup
is nearly free later — but only if the manifest format allows it. Retrofitting
is a format migration.

Restore into a **staging directory**, verify the manifest and object integrity,
then swap — keeping a recoverable pre-restore copy.

---

## 7. What this format does not protect

Stated so the threat model and any user-facing claim stay honest.

- **A compromised host while unlocked.** Decrypted data necessarily exists in
  process memory, SQLCipher pages, the WebView, and image buffers. Zeroizing
  one key does not erase those copies.
- **Swap on a host without encrypted swap.** The app cannot control what the OS
  pages out.
- **Deliberate exports.** CSV, PDF reports and exported photos are plaintext by
  intent, at user-chosen destinations.
- **Loss of both credentials.** There is no reset, no account, no server. This
  is data loss, not a security failure — which is why the recovery ceremony
  forces verification before it will let the user continue.

What it *does* protect: theft or copying of the vault directory, a backup
archive, a synced folder or a disk image, by someone without the passphrase or
recovery key. Permitted leakage in that case: object count, approximate sizes,
filesystem timestamps, and unauthenticated header metadata (format version, KDF
parameters, salts).

---

## 8. Format version

`format_version` is bound into **every object's AEAD context**, so bumping it
invalidates every object and requires re-encrypting the whole vault — 84 GB for
a 5,000-item collection with photos.

Therefore: **frozen for v1.** It exists to make a future format change
*detectable and safe*, not routine. If it ever must change, it requires a
resumable, interruptible re-encryption job that keeps the vault openable
throughout, with old and new versions coexisting. Never bump it in a patch
release.

A header whose `format_version` exceeds what the build supports is refused
clearly rather than partially opened.
