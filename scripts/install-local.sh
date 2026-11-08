#!/bin/sh
# Builds a release binary into ~/.local/bin (replacing any older one), then
# restarts the core the planned way so running terminals pick up the new build.
set -eu
cd "$(dirname "$0")/.."
nice -n 10 cargo build --release
bin="${KOMPITUM_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$bin"
if [ "$(uname -s)" = Darwin ]; then
  app="target/release/Kompitum Core.app"
  rm -rf "$app"
  sh scripts/mac-bundle target/release/kompitum-core "$app"
  bash scripts/mac-sign "$app"
  "$app/Contents/MacOS/kompitum-core" --kom-install-bundle "$app/Contents/MacOS/kompitum-core" "$bin/kompitum-core"
else
  install -m 755 target/release/kompitum-core "$bin/kompitum-core.new"
  mv -f "$bin/kompitum-core.new" "$bin/kompitum-core"
fi
[ -e "$bin/kom" ] || [ -L "$bin/kom" ] || ln -s kompitum-core "$bin/kom"
"$bin/kompitum-core" server restart
