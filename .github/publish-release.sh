#!/usr/bin/env bash
# Replaces a rolling release (stable or bleeding-edge) with the binaries in dist/, and for a stable
# build with a new version number also creates the permanent vX.Y.Z release.
# Inputs (environment): TAG, PRE (true/false), BANNER, BUILD, REPO, SHA, GH_TOKEN.
# Set DRY_RUN=1 to build the notes and files in dist/ and print the gh commands without running them.
set -euo pipefail

gh_release() {
  if [ -n "${DRY_RUN:-}" ]; then
    echo "[dry run] gh release $*"
  else
    gh release "$@"
  fi
}

cp lighthouse.example.toml deploy/systemd/lighthouse.service deploy/update/update-windows.ps1 deploy/update/update-linux.sh dist/
cd dist

stable=true
[ "$PRE" = "true" ] && stable=false

# Stable carries the version in Cargo.toml. Bleeding edge carries the last stable version it came after.
if $stable; then
  version=$(grep -m1 '^version' ../Cargo.toml | cut -d'"' -f2)
else
  version=$(git ls-remote --tags origin 'refs/tags/v*' | sed 's#.*refs/tags/v##' | grep -E '^[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -1 || true)
fi

binaries=$(ls lighthouse-* | grep -v '\.toml$' || true)
# The updaters are listed too: they check the copies they fetch of themselves against this file.
sha256sum $binaries update-windows.ps1 update-linux.sh > SHA256SUMS

# The updater reads this to tell whether a newer build is out on its channel.
{
  printf '{\n  "channel": "%s",\n  "build": %s,\n  "revision": "%s",\n  "version": "%s",\n  "assets": {\n' \
    "$TAG" "$BUILD" "$SHA" "$version"
  first=true
  while read -r hash name; do
    name=${name#\*} # sha256sum marks files read in binary mode with *
    $first || printf ',\n'
    first=false
    printf '    "%s": "%s"' "$name" "$hash"
  done < SHA256SUMS
  printf '\n  }\n}\n'
} > release.json

if $stable; then
  title="Lighthouse $version"
  first_line="Version **$version**, build **$BUILD**, commit \`${SHA:0:12}\`."
elif [ -n "$version" ]; then
  title="Lighthouse ($TAG)"
  first_line="Rolling build **$BUILD**, after version $version, commit \`${SHA:0:12}\`."
else
  title="Lighthouse ($TAG)"
  first_line="Rolling build **$BUILD**, commit \`${SHA:0:12}\`."
fi

# A stable release shows its own section of CHANGELOG.md.
changes=""
if $stable; then
  section=$(awk -v v="## $version" '/^## /{inside = ($0 == v); next} inside' ../CHANGELOG.md | sed -e '/./,$!d')
  changes=$(printf "## What's new in %s\n\n%s\n" "$version" "$section")
fi

download() { # name, label
  printf -- '- **%s:** [%s](https://github.com/%s/releases/download/%s/%s)\n' "$2" "$1" "$REPO" "$TAG" "$1"
}
downloads=$(
  for b in $binaries; do
    case "$b" in
      *windows*) download "$b" "Windows (64-bit)" ;;
      *x86_64-unknown-linux*) download "$b" "Linux (64-bit)" ;;
      *aarch64-unknown-linux*) download "$b" "Linux (ARM, e.g. Raspberry Pi)" ;;
      *apple-darwin*) download "$b" "macOS (Apple silicon)" ;;
      *) download "$b" "$b" ;;
    esac
  done
  download lighthouse.example.toml "Example configuration"
  download lighthouse.service "Linux service file (systemd)"
  download update-windows.ps1 "Updater for older installs (Windows)"
  download update-linux.sh "Updater for older installs (Linux)"
)

commits=$(git log -15 --no-merges --format='- `%h` %s' "$SHA")

{
  echo "$first_line"
  echo
  if [ -n "$changes" ]; then echo "$changes"; echo; fi
  echo "## Downloads"
  echo
  echo "$downloads"
  echo
  echo "Checksums are in \`SHA256SUMS\`."
  echo
  echo "> $BANNER"
  echo
  cat ../.github/release-notes.md
  echo
  echo "## Recent commits"
  echo
  echo "$commits"
} > notes.md

assets=($binaries SHA256SUMS release.json lighthouse.example.toml lighthouse.service update-windows.ps1 update-linux.sh)

gh_release delete "$TAG" --cleanup-tag --yes -R "$REPO" 2>/dev/null || true
if $stable; then flag=--latest; else flag=--prerelease; fi
gh_release create "$TAG" "${assets[@]}" -R "$REPO" --target "$SHA" --title "$title" --notes-file notes.md "$flag"

# A raised version also stays for good as vX.Y.Z; a stable build that keeps the number only refreshes stable.
if $stable && ! git ls-remote --exit-code --tags origin "refs/tags/v$version" >/dev/null 2>&1; then
  sed "s#/download/$TAG/#/download/v$version/#g" notes.md > notes-versioned.md
  gh_release create "v$version" "${assets[@]}" -R "$REPO" --target "$SHA" --title "$title" \
    --notes-file notes-versioned.md --latest=false
fi
