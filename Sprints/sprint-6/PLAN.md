# Sprint 6 – Local vault

Decision taken by the project owner after sprint 5: option 1, a local vault unlocked with a
master passphrase (no OS keychain for now).

## Design

| Topic | Decision |
|-------|----------|
| File | `<workspace>/vault.json`, owner-only. Names in clear, each value sealed separately – listing and referencing work without the passphrase, and one damaged entry does not take the others down |
| Key derivation | Argon2id, 19 MiB / 2 passes / 1 lane (OWASP baseline), 16-byte random salt; parameters stored in the file and bounds-checked on load |
| Encryption | XChaCha20-Poly1305, random 192-bit nonce per value, the secret's name as associated data. A sealed constant distinguishes "wrong passphrase" from "damaged file" |
| Crates | `argon2`, `chacha20poly1305`, `zeroize` (RustCrypto, pure Rust). No home-made primitives |
| Reference | `{{vault:name}}`, resolved by the ordinary variable resolver from a scope that is never serialized – so it works everywhere variables work, including indirectly through an environment variable, and cannot reach scripts |
| Fail closed | `http::prepare` refuses to produce a request that still contains `{{vault:…}}` |
| No leaks | A request that touched the vault gets its snapshot redacted *and* scrubbed by value; history and run reports additionally scrub responses and errors by value |
| Unlocking | CLI: `RSPOSTMAN_VAULT_PASSPHRASE` or hidden prompt, only when something references the vault; never from piped stdin. GUI: backend holds the unlocked vault in memory until *Lock now* / exit |
| GUI surface | Write-only: store, replace, delete, lock, change passphrase. No endpoint returns a secret value |

## Scope

1. `core::vault` (create, unlock, set, get, remove, list, change passphrase, reference scan).
2. Resolver, prepare, engine, history, report integration.
3. CLI: `vault init | status | list | set | get | remove | passwd`, automatic unlock for
   `send`, `request`, `run`, `monitor`, `history replay`, `codegen`, `benchmark`, `profile`, `mock`, `ws`.
4. GUI backend endpoints and vault screen, padlock indicator, "Unlock the vault" action on the
   error shown when a request needs it.
5. Tests at all three levels; documentation of guarantees and limits.

## Out of scope

OS keychain, idle auto-lock, encrypting collections / environments / cookies themselves,
hardware keys, sharing a vault between people.
