# Local builds and publishing (without CI)

`publish-local.sh` builds the committed state on real machines and publishes it to the update server like CI does.
Run it from Git Bash on the Windows PC: `sh packaging/local/publish-local.sh COMMAND`.

- `build [windows|linux|macos|all]`: builds from `git archive HEAD` (never the working tree). Windows here
  (`cargo build --release --locked --bins --features server`, target dir `C:/lr-local-target`), Linux on KT-T480
  (AK-T480 if it is unreachable), macOS on KT-NEO (Intel and Apple Silicon slices, then `universal.sh`). Remote builds
  run detached with a log under `~/lr-local/` and are polled; the results are copied to `dist-local/N/` with the CI file
  names. Run it again to attach to a build that is still running; `FORCE=1` rebuilds.
- `publish [all|windows|linux|macos]`: checks size, sha256 and (Windows) a headless test suite, uploads to the VPS as a
  `.part` file, fixes owner and mode, moves it into `updates/`, then updates the manifest with
  `packaging/update-manifest.mjs`. A platform whose manifest build is higher is skipped.
- `status`: the manifest, the latest CI run and the local builds (and the state of unfetched remote builds).

Build number N: `max(manifest builds, latest build-workflow run) + 1`, or `BUILD=n`. It becomes `GITHUB_RUN_NUMBER`
(the updater installs only a higher build with a different revision); `GITHUB_SHA` is HEAD. `dist-local/N/BUILD.env`
holds N, SHA and VERSION. Environment: `BUILD`, `FORCE=1`, `SKIP_SMOKE=1`, `MAC_HOST`, `LINUX_HOSTS`,
`BUILD_TIMEOUT_MIN` (default 90), `LR_LOCAL_WORK`, `LR_LOCAL_TARGET`.

A later CI run with a lower number cannot downgrade a local publish (`update-manifest.mjs` keeps the higher build), but
CI still replaces the file in `updates/` first, so after such a run publish again. The builders need Rust (rustup), the
Linux packages listed in `packaging/linux/build.sh`, and Xcode tools on the Mac. `~/lr-local/` on a builder can be deleted.
