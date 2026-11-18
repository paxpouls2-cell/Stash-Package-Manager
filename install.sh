#!/usr/bin/env bash
# Builds Stash and installs it for your user (no root needed except to get Rust).
set -euo pipefail
cd "$(dirname "$0")"

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust not found. Installing it from the Arch repos (needs sudo)..."
  sudo pacman -S --needed rust base-devel
fi

cargo build --release

install -Dm755 target/release/stash "$HOME/.local/bin/stash"
mkdir -p "$HOME/.local/share/applications"
sed "s|^Exec=.*|Exec=$HOME/.local/bin/stash|" stash.desktop > "$HOME/.local/share/applications/stash.desktop"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true

echo "Done. Launch 'Stash' from your app menu, or run: $HOME/.local/bin/stash"
