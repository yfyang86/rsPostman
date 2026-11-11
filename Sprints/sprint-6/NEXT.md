# Sprint 6 – Retrospective and backlog

## Done

Secrets can now be kept encrypted at rest and used without ever being written in clear to the
workspace, the terminal or the GUI.

## Backlog

1. Idle auto-lock for the GUI (configurable minutes) and lock on system sleep.
2. Optional OS keychain storage of the vault key on desktops, so the passphrase is typed once.
3. "Move to vault" action on auth fields and secret variables (replace a literal by a
   `{{vault:…}}` reference in one step); a warning when a collection is exported with literal
   credentials.
4. `pm.vault.get()` for scripts behind an explicit per-collection permission, as Postman does.
5. Vault export / import (encrypted) for moving to another machine; today copying `vault.json`
   works and needs the same passphrase.
6. Encrypt the cookie jar with the vault key when the vault is unlocked.
