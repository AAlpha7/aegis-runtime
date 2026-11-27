# Screenshot harness (dev tool — not shipped)

Renders the real `src/vnext` view components with representative fixture props so
the Aurora Glass redesign can be screenshotted for review. It imports production
components and the production CSS cascade (`styles.css` + `design/index.css`); it
is a **sensor**, not a parallel UI.

This is the visual inferential-sensor feed referenced in
`docs/2026-05-31-webapp-aurora-glass-redesign-design.md` §4 — before/after
renders go to Codex review and the owner.

## Usage

```bash
cd webapp
bash preview/build.sh                       # bundles harness.tsx -> preview/bundle.{js,css}
(cd preview && python3 -m http.server 8799) # serve
# screenshot any view at a mobile viewport (Playwright/Chromium):
#   http://localhost:8799/index.html?view=home    (conversation state)
#   http://localhost:8799/index.html?view=empty    (landing state)
```

`bundle.js` / `bundle.css` are generated and git-ignored.
