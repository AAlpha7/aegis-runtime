#!/usr/bin/env bash
# Bundle the screenshot harness. See preview/README.md.
set -euo pipefail
cd "$(dirname "$0")/.."
# import.meta.env is Vite-injected at app build time; the preview bundle must
# define it or any view importing src/api.ts throws at module init.
node_modules/.bin/esbuild preview/harness.tsx \
  --bundle --format=esm --jsx=automatic \
  --define:import.meta.env='{"VITE_API_BASE":""}' \
  --outfile=preview/bundle.js --loader:.svg=dataurl
echo "built preview/bundle.{js,css} — serve: (cd preview && python3 -m http.server 8799)"
