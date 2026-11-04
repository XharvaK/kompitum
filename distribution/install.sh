#!/bin/sh
# Installs limen-core (lc), or updates it:
#
#   curl -fsSL https://releases.limen.invalid/core/install.sh | sh
#
# Into ~/.local/bin/limen-core, with `lc` beside it only if no `lc` exists
# (some window managers ship one). An lc that is already there is swapped
# the planned way (lc update --from): running terminals keep running. One
# from a package manager is left to it. LIMEN_VERSION=0.2.0 picks a version;
# LIMEN_RELEASES_URL points elsewhere (tests).
set -eu

releases=${LIMEN_RELEASES_URL:-https://releases.limen.invalid}
bin=${LIMEN_INSTALL_DIR:-$HOME/.local/bin}

say() { printf '%s\n' "$*"; }
fail() { say "lc: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "needs $1"; }
need curl
need tar

case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  *) fail "lc runs on Linux and macOS for now" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=aarch64 ;;
  *) fail "no build for $(uname -m)" ;;
esac
target="$os-$arch"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

manifest="$releases/core/latest.json"
[ -n "${LIMEN_VERSION:-}" ] && manifest="$releases/core/$LIMEN_VERSION/manifest.json"
curl -fsSL "$manifest" -o "$tmp/manifest.json" || fail "cannot reach $manifest"
# Read without jq: the manifest is flat enough for sed once spaces are gone.
flat=$(tr -d '\n ' < "$tmp/manifest.json")
entry=$(printf '%s' "$flat" | sed -n "s/.*\"$target\":{\([^}]*\)}.*/\1/p")
url=$(printf '%s' "$entry" | sed -n 's/.*"url":"\([^"]*\)".*/\1/p')
want=$(printf '%s' "$entry" | sed -n 's/.*"sha256":"\([^"]*\)".*/\1/p')
version=$(printf '%s' "$flat" | sed -n 's/^{"version":"\([^"]*\)".*/\1/p')
[ -n "$url" ] && [ -n "$want" ] || fail "no build for $target in $manifest"

# Tells the release server's download counter a new install from an update;
# the file is the same either way. Only plain URLs on the release server in
# use are marked: one with a query or fragment (a mirror's signed URL) is
# fetched exactly as the manifest gives it.
src=install
if [ -x "$bin/limen-core" ]; then src=update; fi
case $url in
  *[?#]*) download=$url ;;
  "$releases"/*) download="$url?src=$src" ;;
  *) download=$url ;;
esac
curl -fsSL "$download" -o "$tmp/lc.tar.gz" || fail "download failed: $url"
[ "$(sha256 "$tmp/lc.tar.gz")" = "$want" ] || fail "the download is damaged (checksum mismatch); try again"
tar -xzf "$tmp/lc.tar.gz" -C "$tmp"
"$tmp/limen-core" --version >/dev/null 2>&1 || fail "the downloaded lc does not run on this machine"
# App completion must converge even before the first bundled release exists.
if [ "${LIMEN_BUNDLE_ONLY:-0}" = 1 ] && [ ! -x "$tmp/Limen Core.app/Contents/MacOS/limen-core" ]; then
  exit 0
fi
mkdir -p "$bin"

bundle=0
if [ "$os" = macos ] && [ "$(id -u)" != 0 ]; then
  case "$bin" in "$HOME"/*) bundle=1 ;; esac
fi
if [ "$bundle" = 1 ] && [ -x "$tmp/Limen Core.app/Contents/MacOS/limen-core" ]; then
  action=installed
  [ ! -x "$bin/limen-core" ] || action=updated
  bundled="$tmp/Limen Core.app/Contents/MacOS/limen-core"
  if [ "$action" = updated ]; then
    installed=$("$bin/limen-core" --version)
    candidate=$("$bundled" --version)
    awk -v old="$installed" -v new="$candidate" 'BEGIN {
      sub(/^lc /, "", old); sub(/^lc /, "", new)
      split(old, a, /[.-]/); split(new, b, /[.-]/)
      for (i=1; i<=3; i++) { if (a[i]+0 > b[i]+0) exit 1; if (a[i]+0 < b[i]+0) exit 0 }
    }' || fail "$candidate is older than $installed; use lc update --from with --force to downgrade"
  fi
  case "$releases" in
    https://releases.limen.invalid | https://releases.limen.invalid/)
      /usr/bin/codesign --verify --strict -R '=identifier "dev.limen.core" and anchor apple generic and certificate leaf[subject.OU] = "2W9652Q7QR"' "$tmp/Limen Core.app" || fail "not a signed Limen release" ;;
  esac
  "$bundled" --lc-install-bundle "$bundled" "$bin/limen-core" || fail "installing Limen Core.app failed"
  # A fresh install must not start a core (especially not a system/root core).
  if [ "$action" = updated ]; then
    "$bin/limen-core" server restart >/dev/null || fail "restarting the core failed"
  fi
  say "lc $version $action as Limen Core.app. Recreate pre-update terminals if macOS privacy access is broken; running terminals were kept."
elif [ -x "$bin/limen-core" ]; then
  # Swapped the planned way, so its terminals keep running.
  "$bin/limen-core" update --from "$tmp/limen-core" >/dev/null || fail "updating $bin/limen-core failed (see above)"
  say "lc updated to $version."
else
  cp "$tmp/limen-core" "$bin/limen-core.new"
  chmod 755 "$bin/limen-core.new"
  mv -f "$bin/limen-core.new" "$bin/limen-core"
  say "lc $version installed into $bin/limen-core."
fi
if ! command -v lc >/dev/null 2>&1 && [ ! -e "$bin/lc" ]; then
  ln -s limen-core "$bin/lc"
fi

case ":$PATH:" in
  *":$bin:"*) ;;
  *) say "Add $bin to your PATH to use lc." ;;
esac
say "Agents: lc --skill. People: lc --help."
