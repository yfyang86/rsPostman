# Sprint 6 – Test & fix report

## Regression run

| Layer | Count | Result |
|-------|------:|--------|
| Rust unit tests – +3 vault (round trip, wrong passphrase, tampering, name swap, hostile KDF parameters, nonce uniqueness), +1 resolver, +1 value scrubbing | 54 | pass |
| HTTP / engine integration – +1 (locked → not sent; unlocked → sent; snapshot, script console and history free of the secret) | 6 | pass |
| CLI end-to-end – new `tests/vault.rs` (lifecycle; secrets in requests "and nowhere else": the test greps every file in the workspace) | 11 | pass |
| Playwright – new `vault.spec.js` (create, validation, store, delete, use, lock, wrong passphrase, unlock, reload, change passphrase; greps the workspace on disk) | 35 | pass |
| `cargo clippy --all-targets -- -D warnings`, desktop crate build | – | clean |

## What the tests establish

* `vault.json` contains neither values nor the passphrase; file mode is `0600`.
* A wrong passphrase, a flipped ciphertext bit, two swapped entries and absurd key-derivation
  parameters are all rejected.
* With the vault locked or a secret missing, nothing is sent.
* After using secrets as bearer token, basic-auth password, in an oddly named header, in the
  URL path and in a free-text body – and running a collection with a JSON report – no file in
  the workspace contains a secret value; only `{{vault:name}}` and `<redacted>`.
* `-v --show-secrets` and the GUI's "Show credentials" do not reveal vault secrets.
* Scripts see the placeholder, not the value.

## Defects found while implementing

| # | Found by | Problem | Fix |
|---|----------|---------|-----|
| 1 | design review | Scripts round-trip the variable scopes through JSON; a vault scope would either be handed to the script or lost afterwards | The vault scope is `#[serde(skip)]` and restored after each script |
| 2 | design review | The variable preview endpoint and mock servers resolve variables and would have returned vault values | Both drop the vault before resolving |
| 3 | design review | Name-based redaction would miss a vault secret used in a URL path or free text | Value-based scrubbing for anything that touched the vault |

(Two test failures during the sprint were test artefacts – the test itself had typed the
secret literally into a URL – not product defects.)

## Not verified

* The key derivation cost was not benchmarked on slow hardware; in a release build one unlock takes about
  20 ms on this machine. Debug builds compile `argon2` optimised so tests stay fast.
* The hidden passphrase prompt was checked manually on a pseudo-terminal in sprint 5; the
  automated tests use the environment variable.
* No independent cryptographic review has been done. The construction uses standard,
  maintained primitives in their documented way, but it is new code.
