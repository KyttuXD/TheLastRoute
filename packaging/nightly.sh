#!/bin/sh
# Uploads builds to the rolling "nightly" pre-release of this repository. The release is created on
# first use, its tag is moved to the commit being built, and the files are replaced (--clobber), so
# the release always holds the latest build from main. Needs GH_TOKEN with contents: write (the
# build workflow passes github.token). Usage: sh packaging/nightly.sh FILE...
set -e
TAG=nightly
REPO="$GITHUB_REPOSITORY"

i=0
until gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; do
    i=$((i + 1))
    if [ "$i" -gt 6 ]; then
        echo "could not create the $TAG release" >&2
        exit 1
    fi
    # Another job may create it first; a failed create is retried by the check above.
    gh release create "$TAG" --repo "$REPO" --target "$GITHUB_SHA" --prerelease \
        --title "Nightly build" --notes "Rebuilt on every push to main. The files are replaced in place." >/dev/null 2>&1 || sleep 5
done

# Point the tag at this commit, so the release matches the files uploaded below.
gh api -X PATCH "repos/$REPO/git/refs/tags/$TAG" -f sha="$GITHUB_SHA" -F force=true >/dev/null 2>&1 || true
gh release upload "$TAG" --repo "$REPO" --clobber "$@"
echo "uploaded $* to the $TAG release ($GITHUB_SHA)"
