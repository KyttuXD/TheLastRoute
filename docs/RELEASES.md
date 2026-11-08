# Releases and CI builds

Every build goes to a rolling pre-release in GitHub Releases, not to Actions artifacts. `packaging/nightly.sh` creates the release on first use, moves its tag to the built commit and replaces the files in place.

| Release tag | What | Workflow | When it runs |
|---|---|---|---|
| `nightly` | KYTTU: The Last Route, Windows, Linux, macOS Intel and Apple Silicon | `.github/workflows/build.yml` | Every push to `main` (and by hand) |
| `nightly-server` | `lr_server`, Linux x86_64, with its SHA-256 | `.github/workflows/build.yml` (Linux job) | Every push to `main` |
| `nightly-nw` | Night Watch (`src/bin/night_watch`), Windows, Linux, macOS Intel and Apple Silicon | `.github/workflows/nightly-nw.yml` | By hand only |
| `nightly-lrs` | LR Story (`src/bin/lr_story`), Windows, Linux, macOS Intel and Apple Silicon | `.github/workflows/nightly-lrs.yml` | By hand only |

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
