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
├── vault.header.next     only mid credential change; see §3 "Credential changes"
├── vault.lock            OS file lock held while open; contents informational
├── catalog.db            SQLCipher database (§5)
├── catalog.db-wal        WAL; transient
├── objects/              encrypted originals (§4)
│   └── 3f/7a/3f7a9c02…   random 128-bit object ID, no extension
└── cache/
    └── thumbs/           encrypted derived variants
        └── 9b/21/9b21c7e4…
```

A **backup** is `vault.header` + `catalog.db` + `objects/` + `manifest.json`
(§6). `cache/` is regenerable and excluded. **Omitting `vault.header` makes the
backup permanently undecryptable** — it holds the wrapped data key.

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
> matches; an edited epoch gains nothing, because pairing fails. The retry
> covers at most the 4,096 most recent epochs, so a header edited to claim an
> enormous epoch cannot demand unbounded work.

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

### Credential changes

A passphrase change or recovery-key rotation moves the header *and* the
database to a new `key_epoch` (§5, pairing). One rename cannot replace two
files, so the change is staged:

1. Write the new header to `vault.header.next` (atomically, as above).
2. Advance `key_epoch` in the database. This commit is the point of no return.
3. Rename `vault.header.next` over `vault.header`.

Stopping after any step — a crash, a full disk — leaves a vault that opens. The
old and new headers wrap the *same* data key, so whichever one the typed
credential unwraps reaches the database, and the database's epoch then says
which header is current:

| Stopped after | Database epoch | Unlock with the old credential | Unlock with the new credential |
|---|---|---|---|
| step 1 | old | old header pairs; `.next` is discarded | finishes the change (advances the database, installs `.next`) |
| step 2 | new | `.next` pairs and is installed | `.next` pairs and is installed |

Either way `vault.header.next` is gone afterwards. A header that pairs with
neither is still a pairing error (§5); staging does not loosen that check.

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

`key_epoch` increments on every credential change or rewrap. See
"Credential changes" (§3) for how the two move together.

### Schema versions

| Version | Change |
|---|---|
| 1 | Initial schema |
| 2 | `app_settings` (per-vault settings, including privacy opt-ins) |
| 3 | Effective dates normalized to calendar dates; type categories and the types the forms create; collectibles saved as `generic` reclassified from their fields; `pricing` (`manual`/`market`) and `review_every_days` on assets; FTS index extended to type-specific attributes |
| 4 | `cost_complete` on assets: false when part of a holding was added at an unknown cost (or in another currency), so no gain is computed against a cost covering only some of it. Backfilled from existing `add` events |
| 5 | `firearms` category with `firearm`, `ammunition` and `firearm_accessory` types |
| 6 | `status_events` (dated lost / retired / recovered; existing lost and retired items dated from their last edit) and `cost_statements` (cost replayed in effective-date order from the latest statement; every existing cost recorded as a statement so no figure changes) |
| 7 | `assets.deleted_at` (trash, purged after 30 days), `valuations.voided_at` / `void_reason` (voided values kept, not counted), `asset_revisions` (the record before each edit, last 50 kept) |
| 8 | Attachment details on `asset_media`: `doc_kind` (photo, receipt, appraisal, certificate, warranty, manual, other), `title`, `doc_date`, `note`; only photos can be the cover |
| 9 | `tags` and `asset_tags` (many per asset, case-insensitive names); saved holdings views are stored as a setting |
| 10 | `care_events`: services, repairs, inspections, cleanings, appraisals, batteries and warranties, with provider, cost, an optional linked document and a next-due date |
| 11 | `custody_events`: lent, consigned, at repair, in outside storage, shipped, returned — who, contact, dates, due back, reference, optional document. Contacts never appear in exports or reports |
| 12 | More types: vehicle, boat, home or building, land, electronics, appliance, furniture, tools, sports gear, fashion, books, stamp, toys, stock/bond/fund; categories investments, household, vehicles, property |

---

## 6. Backup and restore

**SQLite's online backup API does not work on an encrypted database** —
SQLCipher returns *"backup is not supported with encrypted databases"*. This is
pinned by a test so a future release lifting the restriction surfaces as a
failure. Pause-and-copy is therefore the only protocol:

1. Pause writes; finish or cancel in-flight imports.
2. `PRAGMA wal_checkpoint(TRUNCATE)` — without this a copy can miss committed
   transactions still living in the WAL.
3. Close all database connections, **keeping the process lock** (`vault.lock`)
   until the vault is reopened: another instance must not open and write to
   the vault mid-copy.
4. Copy `vault.header`, `catalog.db`, and every *live* object the database
   lists into `<destination>.partial`, fsyncing each file.
5. Write the manifest last, then rename `<destination>.partial` to the
   destination. A folder under the real name is therefore always a complete
   backup; a failed one leaves nothing that looks finished.

### Manifest

```json
{
  "format": "asset-manager-vault-v1",
  "manifest_version": 2,
  "format_version": 1, "schema_version": 4,
  "vault_id": "…", "key_epoch": 3, "created_at": "…",
  "objects": [{ "object_id": "3f7a…", "ciphertext_sha256": "…", "bytes": 48213 }]
}
```

The manifest is **plaintext**, so it carries only what a copy of the vault
directory already reveals (§7): object IDs and sizes, plus a SHA-256 of each
*encrypted* file to detect a damaged copy.

> **Never a plaintext hash.** `manifest_version` 1 listed each object's
> plaintext SHA-256 — the deduplication hash. Anyone holding such a backup
> could hash a photograph of their own and confirm whether the vault held
> it, without unlocking anything. Version 1 manifests are still restored
> (the plaintext field is ignored), but are never written. A new backup does
> not change copies already made: delete version 1 backups you no longer need,
> after making a new one.

The manifest carries an object list because objects are immutable and
content-addressed, so incremental backup is nearly free later — but only if
the manifest format allows it.

### Restore

A backup folder is untrusted input, however it was made. Restore:

1. Validates the manifest before anything else: bounded size, known format and
   versions, well-formed vault and object IDs and digests. Header, database and
   objects must be regular files — a symlink is refused.
2. Copies header and database into `<vault>.restore-staging` and **unlocks the
   staged copy with the passphrase or recovery key the backup opens with** —
   the same unlock, migrations and pairing check the restored vault will get.
3. Runs `PRAGMA integrity_check`.
4. Copies every live object **the encrypted database lists** — not the
   manifest, which anyone can edit — checking each one's size against the
   database and its digest against the manifest. From version 2 an object
   missing from the manifest is an error.
5. Only then: refuses if another instance holds the vault's lock, moves the
   current vault aside to `<vault>.pre-restore`, and renames the staged copy
   into place, putting the previous vault back if that rename fails.

A backup that fails any check — damaged, edited, incomplete, or simply one
whose credential is not known — never displaces the vault in place, and its
staging folder is removed. The desktop app keeps the current session open
until step 5, then opens the restored vault with the same credential.

### Verification

A restore rehearsal: everything a restore does (above), into a scratch
folder beside the vault, plus **decrypting every object** — then the scratch
copy is removed. Sizes and digests show a copy is the file that was backed
up; only decryption shows that file is intact and opens under the backup's
key. The result, good or bad, is recorded in the vault's backup history.

### Process lock

`vault.lock` is held with an OS file lock (`flock` / `LockFileEx`) for as long
as the vault is open. The operating system releases it when the process exits
for any reason, so a crash never leaves the vault claimed, on any platform. The
file is left in place on release — deleting it would let one process lock the
old file while another created and locked a new one. Its content (a PID) is
informational only.

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
