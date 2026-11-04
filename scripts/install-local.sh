#!/bin/sh
# Builds a release binary into ~/.local/bin (replacing any older one), then
# restarts the core the planned way so running terminals pick up the new build.
set -eu
cd "$(dirname "$0")/.."
nice -n 10 cargo build --release
bin="${LIMEN_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$bin"
if [ "$(uname -s)" = Darwin ]; then
  app="target/release/Limen Core.app"
  rm -rf "$app"
  sh scripts/mac-bundle target/release/limen-core "$app"
  bash scripts/mac-sign "$app"
  "$app/Contents/MacOS/limen-core" --lc-install-bundle "$app/Contents/MacOS/limen-core" "$bin/limen-core"
else
  install -m 755 target/release/limen-core "$bin/limen-core.new"
  mv -f "$bin/limen-core.new" "$bin/limen-core"
fi
[ -e "$bin/lc" ] || [ -L "$bin/lc" ] || ln -s limen-core "$bin/lc"
"$bin/limen-core" server restart
