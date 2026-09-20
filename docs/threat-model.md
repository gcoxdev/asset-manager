# Threat model

What Asset Manager protects, what it does not, and why. Modelled on
[QiRing's threat model](https://github.com/gcoxdev/QiRing), which documents
residual risk rather than claiming completeness.

This document is a commitment. If a claim here is not true of the code, the
code is the bug — and a claim that cannot be kept should be removed from this
document rather than quietly weakened.

Status: current as of 2026-09-20, pre-release.

---

## 1. What is being protected

A catalogue of valuables: what you own, what it is worth, **where it is
stored**, and photographs of it.

That combination is unusual. Most password managers protect credentials, which
are useful only to someone who also knows where to use them. This protects an
itemised inventory with locations — which is, in the wrong hands, a shopping
list with a map. The storage-location field is the reason the encryption
posture is not optional.

---

## 2. What is protected

**Theft or copying of the vault**, by someone without the passphrase or
recovery key:

- the vault directory copied from a running or powered-off machine
- a backup archive, on an external drive or in cloud storage
- a synced folder (Dropbox, Drive, Syncthing)
- a disk image, or a recovered deleted file
- a laptop stolen while the vault is **locked**

In each case the attacker gets ciphertext. Asset names, notes, storage
locations, valuations, photographs and thumbnails are all encrypted.

**Enforced by:** Argon2id (64 MiB, 3 iterations) deriving a key-encryption key
that wraps a random data key; XChaCha20-Poly1305 over 64 KiB chunks for
objects; SQLCipher for the database; HKDF-SHA256 domain separation so database,
object and thumbnail keys are distinct.

**Verified by:** integration tests that plant high-entropy sentinels in asset
names, notes, storage locations and image bytes, exercise search and
thumbnailing, then scan every file in the vault — including the FTS index and
the WAL — asserting no sentinel appears. A further test copies a whole vault
and asserts it cannot be opened with any wrong credential.

---

## 3. Permitted leakage

Visible to someone holding the vault directory, and deliberately not hidden:

| Observable | Why it is not hidden |
|---|---|
| Number of objects | Hiding it needs padding files that never shrink |
| Approximate object sizes | Same; a 4 MB photo is distinguishable from a 40 KB one |
| Total vault size | Follows from the above |
| Filesystem timestamps | Owned by the OS, not the app |
| Vault ID, format version, KDF parameters, salts | Must be readable before any key exists |
| That the vault *is* an Asset Manager vault | The header names the format |

Object **filenames** leak nothing: they are random 128-bit IDs with no
extension. The plaintext hash used for deduplication lives only inside the
encrypted database, so an attacker cannot confirm whether a specific image is
present by hashing their own copy.

---

## 4. What is NOT protected

Stated plainly, because a security claim that overreaches is worse than a
narrow one.

### 4.1 A compromised machine while the vault is unlocked

Decrypted data necessarily exists in process memory, SQLCipher page cache, the
WebView renderer, JavaScript strings and image buffers. Another process running
as the same user can read that memory or capture the passphrase as it is typed.

Keys are held in `Zeroizing` buffers and wiped on lock, but that cannot erase
copies made by the WebView, the allocator, or the GPU.

**Mitigation available to you:** auto-lock after 15 minutes idle is on by
default, and locking drops the key, closes the database and releases the
process lock.

### 4.2 Swap on a host without encrypted swap

The application cannot control what the OS pages to disk. On a machine with
unencrypted swap, decrypted fragments may reach it while the vault is unlocked.

**Mitigation available to you:** encrypted swap, or full-disk encryption.
Neither is required to use the app, and neither is a substitute for the app's
own encryption — FDE protects a powered-off machine only.

### 4.3 Anything you deliberately export

CSV exports, insurance reports, printed recovery sheets and photos you export
are **plaintext by intent**, at destinations you choose. Once written they are
outside the vault entirely. The app warns before writing a CSV export and
states that it is neither encrypted nor a backup.

### 4.4 A weak passphrase

Argon2id at 64 MiB makes guessing expensive, not impossible. A passphrase found
in a wordlist will fall to an offline attack against a stolen vault. The
minimum is 12 characters, enforced in the backend, which is a floor rather than
a recommendation.

### 4.5 Loss of both credentials

There is **no password reset and no recovery backdoor**. No account, no server,
nobody with a master key — including the author. Losing both the passphrase and
the recovery key means the catalogue is unrecoverable.

This is a design property, not an oversight. The recovery ceremony requires
acknowledging the key and retyping its last six characters before it will let
you continue, specifically to stop people clicking past it.

### 4.6 A malicious or compromised build

If the binary you run has been tampered with, nothing here applies. Releases
should be verified against published checksums. Reproducible builds are not
yet implemented.

### 4.7 Targeted forensic recovery of deleted data

Deleting a photo unlinks its ciphertext and purges its thumbnails, but does not
overwrite the underlying blocks. On an SSD, wear levelling makes overwriting
unreliable anyway. Recovered blocks are still ciphertext, so this matters only
in combination with a compromised key.

---

## 5. Network exposure

The application works fully offline. All network access is **optional,
manual, and initiated by you** — nothing is fetched in the background.

| Feature | Discloses | To whom | Default |
|---|---|---|---|
| Metals prices | Your IP, that you asked for metal prices | metals.dev | Off; needs a key |
| Crypto prices | Your IP, **which coins you asked about** | CoinGecko | Off; needs a key |
| Watch-only balances | Your IP **and the addresses you query** | block explorer | Off; separate opt-in, not yet implemented |

The middle row is worth reading twice: asking for prices reveals which assets
you hold, even though it does not reveal how much. The third is gated behind a
*separate* opt-in precisely because accepting price fetching is not the same as
accepting address disclosure.

API keys are stored in the **OS keyring**, never in the vault or a
configuration file. Error messages are scrubbed of keys before display, since
HTTP client errors can include the request URL.

The WebView runs with `incognito: true` and a Content-Security-Policy that
permits no remote origins. Decrypted images are served over a custom protocol
with `Cache-Control: no-store`, so the WebView does not cache plaintext to
disk.

---

## 6. Trust boundaries

| Boundary | Crossing it requires |
|---|---|
| Disk → process | The passphrase or recovery key |
| Process → WebView | A Tauri command or the `asset://` protocol; both reject while locked |
| WebView → disk | Nothing direct; the WebView has no filesystem access |
| Process → network | An explicit user action and a configured API key |
| Another process → vault | The passphrase; or reading this process's memory while unlocked |

The `asset://` protocol is treated as a **separate authorization surface** from
Tauri commands, because the WebView reaches it directly. Object IDs must be
exactly 32 hex characters, which makes path traversal impossible by
construction rather than by sanitisation, and only known thumbnail variants are
servable.

---

## 7. Residual risks

Known, accepted, and unresolved:

- **JavaScript strings cannot be zeroized.** A passphrase typed into the
  unlock field exists in WebView memory beyond our control until garbage
  collection, if then.
- **Tamper detection requires the correct credential.** The vault header is
  authenticated through the AAD of the wrapped key, so a modified header is
  indistinguishable from a wrong passphrase until the right one is entered.
  Both report the same error deliberately — distinguishing them would tell an
  attacker editing the header whether they had guessed the passphrase.
- **Changing the passphrase is not key rotation.** An older backup still holds
  a wrapper the old passphrase opens, and the data key is unchanged, so the old
  passphrase plus an old backup decrypts newer content. Genuine rotation
  requires re-encrypting everything and is not implemented.
- **No integrity check across the vault as a whole.** Individual objects and
  database pages are authenticated, but an attacker who deletes an object file
  causes a missing-photo error rather than a tamper alarm.
- **Dependency risk.** WebKitGTK, SQLCipher and the Rust crypto crates are
  trusted. `cargo audit` runs in CI; a vulnerability in a transitive
  dependency is still a vulnerability here.
- **Icons are placeholders** copied from QiRing and must be replaced before
  any public release.
- **No reproducible builds**, so a published binary cannot yet be verified
  against its source by a third party.

---

## 8. Reporting a problem

This is pre-release software with no formal security process. If you find a
vulnerability, open an issue describing the impact without a working exploit,
or contact the maintainer directly.

Do not use this as the only copy of information you cannot afford to lose.
Keep independent backups.
