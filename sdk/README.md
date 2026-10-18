# dltool-sdk

[![npm](https://img.shields.io/npm/v/dltool-sdk.svg)](https://www.npmjs.com/package/dltool-sdk)

Browser helper for [Download Tool](https://github.com/wlfei0502/download-tool): small files use a normal browser download; large files go to the desktop app on `127.0.0.1:29128` (or `dltool://` / an install page).

## Install

```bash
npm install dltool-sdk
```

```ts
import { download } from "dltool-sdk";
```

**Script / CDN** (globals: `DownloadToolSdk`, alias `DownloadTool`):

```html
<script src="https://unpkg.com/dltool-sdk/dist/dltool-sdk.iife.js"></script>
<!-- or https://cdn.jsdelivr.net/npm/dltool-sdk/dist/dltool-sdk.iife.js -->
<script>
  DownloadToolSdk.download({
    downloadUrl: "https://cdn.example.com/file.zip",
    fileName: "file.zip",
    size: 1234567890,
    config: { installUrl: "https://your-host/download-tool" },
  }).then(console.log);
</script>
```

## Usage

```ts
import { download } from "dltool-sdk";

// Direct URL (no ticket API):
const result = await download({
  downloadUrl: "https://cdn.example.com/file.zip",
  fileName: "file.zip",
  size: 1234567890,
  config: {
    sizeThreshold: 104857600,
    installUrl: "https://your-host/download-tool",
    localPort: 29128,
  },
});

// Or look up via fileId + website ticket API:
await download({
  fileId: "f_01hxyz",
  size: 1234567890,
  apiBase: "https://your-host",
  config: { /* ... */ },
  onNeedInstall() {
    // Show the site's own install prompt.
  },
});
```

`DownloadTool.download` is the same function.

## Build from source

```bash
cd sdk
npm install
npm run build
```

Outputs:

- `dist/index.js` / `dist/index.cjs` — bundlers (`import` / `require`)
- `dist/index.d.ts` — TypeScript types
- `dist/dltool-sdk.iife.js` — `<script>` / CDN

## Order

1. Resolve the file: if `downloadUrl` + `fileName` are provided, use them; otherwise `POST {apiBase}/api/download-ticket` with `{ fileId }`. The URL must be `http://` or `https://` and is passed through unchanged.
2. If `size < sizeThreshold` (default **104857600**), click an `<a download>` for that URL and return `{ mode: "browser" }`.
3. Otherwise `GET http://127.0.0.1:{localPort}/health` with a **400ms** timeout. The default port is **29128**.
4. Health ready (`ok: true` and `aria2` is not `false`): `POST /v1/tasks` with camelCase `{ url, fileName, size, referrer, userAgent }` and return `{ mode: "local", taskId }`.
5. Connection error, or a health body that is not ready: set `window.location.href` to `{protocol}://add?url=...&fileName=...&size=...` (default scheme `dltool`), then poll `/health` for about **2s**.
6. If that poll sees the app, return `{ mode: "protocol" }`. If the app is still unreachable, call `onNeedInstall` when it was provided. If it was omitted and `installUrl` is set, open that URL. Return `{ mode: "install" }`.
7. A first-probe timeout uses that same install path and does not open the protocol URL.

`referrer` comes from `document.referrer` (or the page URL). `userAgent` comes from `navigator.userAgent`. Empty values are left off the task body and the protocol URL.

## Tests

```bash
cd sdk && npm test
```

Requires Node **18+**. `fetch` and `window` are mocked.
