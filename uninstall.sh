#!/usr/bin/env bash
# Removes the Stash binary and menu entry from your user account.
set -euo pipefail

rm -f "$HOME/.local/bin/stash"
rm -f "$HOME/.local/share/applications/stash.desktop"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true

echo "Stash removed."
