#!/bin/sh
# Installs the Limen app, and limen-core if there's none yet:
#
#   curl -fsSL https://releases.limen.invalid/app/install.sh | sh
#
# Linux: the app into ~/.local/share/limen-app, `limen-app` in ~/.local/bin,
# a launcher entry and icon. macOS: Limen.app into ~/Applications, keeping
# its release signature intact (ad hoc for unsigned/dev builds). The core, with its
# own installer, into ~/.local/bin/limen-core (and `lc`) only if none is
# there yet; the app
# keeps it current after that. Running it again updates the app.
set -eu

releases=${LIMEN_RELEASES_URL:-https://releases.limen.invalid}
bin="$HOME/.local/bin"

say() { printf '%s\n' "$*"; }
fail() { say "limen: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "needs $1"; }
need curl
need tar

case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  MINGW* | MSYS* | CYGWIN*) fail "this installer is for Linux and macOS; on Windows, run this in PowerShell: irm $releases/app/install.ps1 | iex" ;;
  *) fail "no build for $(uname -s)" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=aarch64 ;;
  *) fail "no build for $(uname -m)" ;;
esac
if [ "$os" = macos ] && [ "$arch" = x86_64 ]; then
  # Rosetta reports x86_64 even on Apple Silicon.
  if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ] ||
     [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
    arch=aarch64
  else
    fail "Intel Macs aren't supported; build from source: https://github.com/herdrdev/herdr-app#build-from-source"
  fi
fi
target="$os-$arch"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$releases/app/latest.json" -o "$tmp/latest.json" || fail "cannot reach $releases"
# Read without jq: the manifest is flat enough for sed once spaces are gone.
entry=$(tr -d '\n ' < "$tmp/latest.json" | sed -n "s/.*\"$target\":{\([^}]*\)}.*/\1/p")
url=$(printf '%s' "$entry" | sed -n 's/.*"url":"\([^"]*\)".*/\1/p')
want=$(printf '%s' "$entry" | sed -n 's/.*"sha256":"\([^"]*\)".*/\1/p')
version=$(tr -d '\n ' < "$tmp/latest.json" | sed -n 's/^{"version":"\([^"]*\)".*/\1/p')
[ -n "$url" ] && [ -n "$want" ] || fail "no build for $target yet"

# Tells the release server's download counter a new install from a reinstall;
# the file is the same either way. Only plain URLs on the release server in
# use are marked: one with a query or fragment (a mirror's signed URL) is
# fetched exactly as the manifest gives it.
src=install
for have in "${XDG_DATA_HOME:-$HOME/.local/share}/limen-app" "$HOME/Applications/Limen.app" /Applications/Limen.app; do
  if [ -e "$have" ]; then src=reinstall; fi
done
case $url in
  *[?#]*) download=$url ;;
  "$releases"/*) download="$url?src=$src" ;;
  *) download=$url ;;
esac

say "Installing Limen ${version:-} for $target..."
curl -fsSL "$download" -o "$tmp/app.tar.gz" || fail "download failed"
[ "$(sha256 "$tmp/app.tar.gz")" = "$want" ] || fail "download is damaged (checksum mismatch); try again"
tar -xzf "$tmp/app.tar.gz" -C "$tmp"
mkdir -p "$bin"

if [ "$os" = linux ]; then
  data=${XDG_DATA_HOME:-$HOME/.local/share}
  home="$data/limen-app"
  mkdir -p "$data"
  rm -rf "$home.new"
  mv "$tmp/limen-app" "$home.new"
  rm -rf "$home"
  mv "$home.new" "$home"
  ln -sfn "$home/limen-app" "$bin/limen-app"
  mkdir -p "$data/applications" "$data/icons/hicolor/scalable/apps"
  cp "$home/limen-app.svg" "$data/icons/hicolor/scalable/apps/limen-app.svg"
  cat > "$data/applications/limen-app.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Limen
Comment=Terminals and coding agents that keep running
Exec=$bin/limen-app
Icon=limen-app
Terminal=false
Categories=Development;
StartupWMClass=limen-app
EOF
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$data/applications" 2>/dev/null || true
  [ -f "$data/icons/hicolor/icon-theme.cache" ] && gtk-update-icon-cache -f -t "$data/icons/hicolor" >/dev/null 2>&1 || true
else
  apps="$HOME/Applications"
  app="$apps/Limen.app"
  mkdir -p "$apps"
  # Prepared in the temp folder, so Spotlight never indexes a half-made
  # bundle under a temporary name in Applications.
  # Never change a certificate-signed bundle: macOS permissions follow its
  # signing identity. Only unsigned/ad-hoc development builds need this PATH.
  signed=false
  if signature=$(/usr/bin/codesign -dvv "$tmp/Limen.app" 2>&1); then
    if printf '%s\n' "$signature" | grep -q '^Authority=Developer ID Application:'; then
      signed=true
    elif ! printf '%s\n' "$signature" | grep -qx 'Signature=adhoc'; then
      fail "cannot determine app signature: $signature"
    fi
  elif ! printf '%s\n' "$signature" | grep -q ': code object is not signed at all$'; then
    fail "cannot determine app signature: $signature"
  fi
  if [ "$signed" = true ]; then
    /usr/bin/codesign --verify --deep --strict "$tmp/Limen.app" || fail "invalid app signature"
  else
    /usr/libexec/PlistBuddy -c "Add :LSEnvironment:PATH string $bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin" "$tmp/Limen.app/Contents/Info.plist"
    /usr/bin/codesign --force --sign - "$tmp/Limen.app" >/dev/null 2>&1
  fi
  rm -rf "$app"
  mv "$tmp/Limen.app" "$app"
  ln -sfn "$app/Contents/MacOS/Limen" "$bin/limen-app"
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app" 2>/dev/null || true
fi

# The core: installed only if there's none yet. One that's already there
# (yours, or from a package manager) is left alone; the app updates it.
if [ ! -e "$bin/limen-core" ] && ! command -v limen-core >/dev/null 2>&1; then
  curl -fsSL "$releases/core/install.sh" | LIMEN_INSTALL_DIR="$bin" LIMEN_RELEASES_URL="$releases" sh || say "limen-core didn't install; Limen will offer it when it opens."
fi

say "Limen is installed. Open it from your apps, or run: limen-app"
case ":$PATH:" in
  *":$bin:"*) ;;
  *) say "Add $bin to your PATH to use limen-app and lc from a terminal." ;;
esac
