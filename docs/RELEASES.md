# Releases and CI builds

Every build goes to a rolling pre-release in GitHub Releases, not to Actions artifacts. `packaging/nightly.sh` creates the release on first use, moves its tag to the built commit and replaces the files in place.

| Release tag | What | Workflow | When it runs |
|---|---|---|---|
| `nightly` | KYTTU - The Last Route, Windows, Linux, macOS Intel and Apple Silicon, and the legacy Mac app for 64-bit Intel Macs on OS X 10.5 to 10.11 (`last_route-macos-legacy.zip`, job `macos-legacy`; runs on a Mac Pro 3,1 under 10.6.8) | `.github/workflows/build.yml` | Every push to `main` (and by hand: `gh workflow run build.yml`) |
| `nightly-server` | `lr_server`, Linux x86_64, with its SHA-256 | `.github/workflows/build.yml` (Linux job) | Every push to `main` |
| `nightly-nw` | Night Watch (`src/bin/night_watch`), Windows, Linux, macOS Intel and Apple Silicon | `.github/workflows/nightly-nw.yml` | By hand only |
| `nightly-lrs` | LR Story (`src/bin/lr_story`), Windows, Linux, macOS Intel and Apple Silicon | `.github/workflows/nightly-lrs.yml` | By hand only |

The `nightly` release also holds the launcher (docs/LAUNCHER.md), built by the same jobs on every push to `main`:
`lr_launcher-windows-x86_64.exe`, `lr_launcher-linux-x86_64` and `lr_launcher-macos-universal.zip` (an `.app`). An existing
device installs it once from there; after that the launcher keeps the game current from the same release. These steps are
`continue-on-error`, so a launcher failure never stops the game builds from publishing. The macOS one has not run on a Mac yet.
A launcher published before `crates/lr_update/keys/trusted.txt` has a key cannot install anything.

The `nightly` release also holds the signed native build the launcher and the in-game updater download, as flat files per platform
(Windows and Linux x86_64 from CI): `native-<platform>-latest.json` and, for the newest 2 builds,
`native-<platform>-builds-<N>-manifest.json`, `...-manifest.json.sig` and `...-f0.z`, `f1.z` and so on. They are written by
`packaging/native/publish-github.sh` (secret `LR_NATIVE_SIGNING_KEY`, step `continue-on-error`); see
[LAUNCHER.md](LAUNCHER.md#github-nightly-as-the-source). Do not delete them by hand.

The legacy Mac app is built by its own job with an old compiler ([MACOS_SNOW_LEOPARD.md](MACOS_SNOW_LEOPARD.md)). That job and
its renderer check may fail without stopping the other builds from being published. The old `nightly-legacy`
release is no longer written to.

## Running a manual build

1. GitHub, Actions tab, pick `nightly-nw` or `nightly-lrs` in the list on the left.
2. Run workflow, branch `main`, Run workflow.
3. When every job is green, the files are on the release page: `https://github.com/KyttuXD/TheLastRoute/releases/tag/nightly-nw` or `.../tag/nightly-lrs`.

Or from a terminal with the GitHub CLI:

```bash
gh workflow run nightly-nw.yml --repo KyttuXD/TheLastRoute
```

```bash
gh workflow run nightly-lrs.yml --repo KyttuXD/TheLastRoute
```

## Rules all the release workflows follow

- The same Rust toolchain as `build.yml` (`RUST_TOOLCHAIN`), and `Swatinem/rust-cache` with a key per job.
- File names are `<bin>-<os>-<arch>` (`.exe` on Windows, `.tar.gz` on Linux, `.zip` on macOS).
- The tag is a pre-release and is moved to the commit that was built; nothing else is tagged.
- No AI names in release titles or notes, and no attribution lines (see `AGENTS.md`).

Per-game build notes: [Night Watch](../src/bin/night_watch/docs/BUILD.md), [LR Story](../src/bin/lr_story/docs/BUILD.md).
