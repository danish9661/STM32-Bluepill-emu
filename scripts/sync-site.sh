#!/usr/bin/env bash
# Sync the rebuilt wasm + JS glue from pkg/ into site/ (GitHub Pages source).
# Run after every `wasm-pack build`, then commit site/ so the live demo
# actually serves the new wasm (a stale site/*.wasm silently ships old code).
set -euo pipefail
cd "$(dirname "$0")/.."

copied=0
for f in emulator.js stm32_bluepill_wasm.js stm32_bluepill_wasm_bg.wasm unicorn_arm.js; do
    if [ -f "pkg/$f" ]; then
        cp "pkg/$f" "site/$f"
        echo "synced site/$f"
        copied=1
    fi
done

if [ "$copied" = "1" ]; then
    echo
    echo "Done. Commit the changes and push to publish the live demo."
    echo "  git add site/ && git commit -m 'site: sync rebuilt wasm' && git push origin master"
else
    echo "no pkg files found — did you run wasm-pack build first?" >&2
    exit 1
fi
