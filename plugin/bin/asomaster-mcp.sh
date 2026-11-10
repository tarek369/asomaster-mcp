#!/bin/sh
# Starts the ASOMaster MCP bridge (asomaster-mcp) that ships inside the ASOMaster desktop app.
# This script only looks for the installed app and runs its bridge; it downloads nothing.
# The bridge's source is the src/ folder of this repository.
set -eu

for bin in \
  "/Applications/ASOMaster.app/Contents/MacOS/asomaster-mcp" \
  "$HOME/Applications/ASOMaster.app/Contents/MacOS/asomaster-mcp" \
  "${LOCALAPPDATA:-}/ASOMaster/asomaster-mcp.exe"
do
  if [ -x "$bin" ]; then
    exec "$bin" "$@"
  fi
done

echo "ASOMaster is not installed. Download it from https://asomaster.app, activate a license, then restart Claude." >&2
exit 1
