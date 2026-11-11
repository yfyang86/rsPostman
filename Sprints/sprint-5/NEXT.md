# Sprint 5 – Retrospective and open decisions

## Done

The contributed profiler / benchmark was reviewed and corrected (measurement method, error
accounting, GUI bugs, tests), and credentials are now masked on screen, kept off the command
line and out of history, reports and logs.

## Open decision: encryption at rest

Secrets that the user stores (environment values, auth fields typed literally, cookies) are
still plain JSON, protected by file permissions only. Options, for the owner to choose:

1. **Local vault** (`{{vault:name}}`), encrypted with a key derived from a master passphrase
   (Argon2id + XChaCha20-Poly1305). Portable and CI-friendly (`RSPOSTMAN_VAULT_PASSPHRASE`),
   but adds an unlock step to the GUI.
2. **OS keychain** (Keychain / Credential Manager / Secret Service) for secret-typed
   variables. No passphrase, but needs a desktop session on Linux and is not portable.
3. Encrypt whole environment files. Simple to explain, breaks Postman file compatibility.

Recommendation: option 1 with option 2 as the place to keep the vault key on desktops.

## Smaller follow-ups

* "Export without secrets" for environments (blank out secret-typed values).
* A real streaming test server for the profiler (the built-in HTTP server writes responses
  in one piece).
* Prune finished run / benchmark / profile jobs from the GUI backend's memory.
