# Sprint 8 – Test & fix report

## Regression run

| Layer | Count | Result |
|-------|------:|--------|
| Rust tests – +2 in `crates/server`: every screenshot the manual references is shipped and every shipped screenshot is used; every in-page link resolves to a heading; every chapter a help button points at exists; the help documents and images are served and path traversal is refused | 82 | pass |
| Playwright – new `help.spec.js` (3): topics, screenshots that actually load, previous / next, in-text links, search with and without results; F1, the ? button and seven tool pages open the right chapter in one shared tab; Markdown text cannot inject markup or script links | 40 | pass |
| `cargo clippy --all-targets -- -D warnings`, desktop crate build | – | clean |
| `node scripts/screenshots.js` | 24 images | regenerates from scratch |

Screenshots were reviewed by eye after generation.

## Defects found and fixed

| # | Found by | Problem | Fix |
|---|----------|---------|-----|
| 1 | reviewing the first screenshots | **Top bar overflow.** At window widths around 1200 px the toolbar pushed the edit-environment, help and theme buttons off-screen (the vault and help buttons added in sprints 6 and 8 made it worse) | The toolbar may shrink and clips before the right-hand controls do; below 1340 px the tools show icons only |
| 2 | Playwright renderer test | A Markdown link with a `javascript:` target would have become a clickable link | Such links are rendered as plain text. (The shipped documents contain none; this guards future edits) |
| 3 | reviewing screenshots | The params screenshot cut off the path-variable rows | That shot uses a taller window |

## Manual accuracy

Labels, menu entries, option names and commands in the manual were written against the
running application and the CLI `--help` output. Statements about behaviour repeat what
earlier sprints tested. Not independently re-verified for this sprint: every single row of
the troubleshooting table.

## Cost

The screenshots add about 5 MB to the repository and to the `rspostman` binary (they are
embedded so the Help Center works offline).
