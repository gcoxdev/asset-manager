# Security policy

Asset Manager keeps an itemized list of what someone owns and where it is
stored. Security reports are taken seriously and handled privately.

## Reporting a vulnerability

Please **do not** open a public issue. Report privately through GitHub's
"Report a vulnerability" button on the repository's Security tab (private
vulnerability reporting must be enabled in the repository settings).

Include what you found, how to reproduce it, and the version or commit. A
first response comes within a week. Please allow time for a fix and a
release before disclosing publicly; you will be credited unless you would
rather not be.

## Scope

In scope: anything that defeats the guarantees in
[docs/threat-model.md](docs/threat-model.md) — reading vault or backup
contents without a credential, learning more than the stated permitted
leakage, bypassing the lock, a crafted vault/backup/CSV/image that corrupts
data or escapes its intended effect, or a release artifact that does not
match its published checksum.

Out of scope, as the threat model states: an attacker who already controls
the computer while the vault is unlocked, unencrypted swap, files the owner
deliberately exported, and weak passphrases.

## Supported versions

Until a 1.0 release, only the latest release receives security fixes.
