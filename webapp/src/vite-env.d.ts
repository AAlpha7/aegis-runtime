/// <reference types="vite/client" />

/** Build-time version string injected by vite.config.ts `define`
 *  (format `vYYYYMMDD.<shortSha>`). Absent in non-vite test builds. */
declare const __APP_VERSION__: string;
