# Local CI with localci

[localci](https://github.com/peters/localci) runs this repo's GitHub Actions workflows (`.github/workflows/*.yml`) on
this PC, so a change can be checked before any push. GitHub CI stays the final check.

Installed at `C:\Users\Kyttu\AppData\Local\Programs\localci\localci.exe` (release v0.3.1, SHA-256 checked).

## Everyday commands

Run these from the repo root:

    localci list -w test                  # the jobs in test.yml and where each runs
    localci plan -w test                  # the steps that would run, and why others are skipped
    localci run -w test "test (windows-latest)"   # the Windows test job, here
    localci run -w build "build"          # a build job by id (use the job's id from list)
    localci check                         # can localci read every workflow in this repo

Logs are kept per step; the path is printed at the end of a run. Add `--json -q` for a machine-readable report.

## What runs here and what does not

- Steps that use marketplace actions (`actions/checkout`, `dtolnay/rust-toolchain`, `Swatinem/rust-cache`,
  `actions/upload-artifact`) are skipped. This PC already has the checkout and Rust.
- Steps under `if: runner.os == 'Linux'` run only on Linux jobs, so the Windows job skips them.
- Jobs whose `runs-on` is another OS are not run here (the Linux and macOS jobs run only on their own machines).
- Nightly publishing (`packaging/nightly.sh`) needs `GH_TOKEN` and runs only on `main` in GitHub; it is skipped here.

## Rules

- Run the job for each platform you changed before a push: the Windows test job for Rust changes,
  and `localci run --fast` for quick repeat checks (it reuses results and skips unchanged build steps).
- A push is for the final GitHub check, not for finding a failing test.
- Do not add `--fast` results to a commit message as proof: run the full job once before a release push.
