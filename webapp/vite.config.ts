import { execFileSync } from 'node:child_process';
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Build-time version stamp so a deploy is verifiable from the UI footer.
// Format: vYYYYMMDD.<shortSha>. Overridable via VITE_APP_VERSION; a git failure
// falls back to a date-only stamp (still changes daily).
function buildVersion(): string {
  if (process.env.VITE_APP_VERSION) return process.env.VITE_APP_VERSION;
  const d = new Date();
  const ymd = `${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, '0')}${String(d.getDate()).padStart(2, '0')}`;
  try {
    const sha = execFileSync('git', ['rev-parse', '--short', 'HEAD'], {
      stdio: ['ignore', 'pipe', 'ignore'],
    })
      .toString()
      .trim();
    return `v${ymd}.${sha}`;
  } catch {
    return `v${ymd}.local`;
  }
}

export default defineConfig({
  plugins: [react()],
  base: '/ui/',
  define: {
    __APP_VERSION__: JSON.stringify(buildVersion()),
  },
  build: {
    outDir: 'dist',
    sourcemap: false,
    // Keep bundle small for fast Mini App loading
    rollupOptions: {
      output: {
        hashCharacters: 'hex',
        manualChunks: undefined,
      },
    },
  },
  server: {
    port: 3000,
  },
});
