# QA checker

Run before building, from the repo root:

    node tools/qa/check.mjs            # checks git-changed and untracked files
    node tools/qa/check.mjs a.rs b.json  # checks only the listed files for EOL and debug macros

Checks:
- Every JSON under `src/bin/last_route/data`, `src/bin/last_route/assets/hunters` and `data/regions` parses, with no tabs outside strings.
- Changed or listed files (plus all those JSON files) do not mix CRLF and LF.
- Changed Rust files have no `println!` or `dbg!` (comments and strings ignored; `bin/night_watch`, `bin/lr_server` and test/build files are skipped).
- Every `include_str!` / `include_bytes!` path resolves to an existing file.

Exit code 1 on any problem. No dependencies, Node 18+. The helpers are exported for tests.
