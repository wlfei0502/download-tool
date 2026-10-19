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

`onNeedInstall` works for **both** direct URL and ticket/`fileId` flows. It runs when the file is at or above `sizeThreshold` and the desktop app is still unreachable (after `dltool://` or a first-probe timeout). Small files never call it — they stay in the browser.

```ts
import { download } from "dltool-sdk";

function offerOsInstaller() {
  // Host these files yourself (names match scripts/package-*.sh|.ps1 outputs).
  const INSTALLERS = {
    mac: "https://your-cdn.example/releases/download-tool.dmg",
    win: "https://your-cdn.example/releases/download-tool-portable.zip",
    linux: "https://your-cdn.example/releases/download-tool-linux.tar.gz", // optional; not shipped yet
  };

  const ua = navigator.userAgent;
  const platform = navigator.platform || "";
  let url = INSTALLERS.linux;
  if (/Mac|iPhone|iPad|iPod/i.test(ua) || /Mac/i.test(platform)) {
    url = INSTALLERS.mac;
  } else if (/Win/i.test(ua) || /Win/i.test(platform)) {
    url = INSTALLERS.win;
  }

  // Prefer a temporary <a> so this tab is not navigated away.
  // (window.location.href = url would replace the current page.)
  const a = document.createElement("a");
  a.href = url;
  a.rel = "noopener";
  a.download = ""; // hint; ignored cross-origin, but still starts a download for many CDNs
  document.body.appendChild(a);
  a.click();
  a.remove();

  // Or open a dedicated install page without leaving this page:
  // window.open("https://your-host/download-tool", "_blank", "noopener");
}

// Direct URL (no ticket API) — same onNeedInstall:
const result = await download({
  downloadUrl: "https://cdn.example.com/file.zip",
  fileName: "file.zip",
  size: 1234567890,
  config: {
    sizeThreshold: 104857600,
    localPort: 29128,
  },
  onNeedInstall: offerOsInstaller,
});

// Or look up via fileId + website ticket API:
await download({
  fileId: "f_01hxyz",
  size: 1234567890,
  apiBase: "https://your-host",
  onNeedInstall: offerOsInstaller,
});
```

`DownloadTool.download` is the same function.

When `onNeedInstall` is set, the SDK **does not** open `installUrl` — your callback owns the UX. Omit the callback and set `config.installUrl` if you prefer a single install page with links for every OS (see [examples/install-page](../examples/install-page/index.html)).

**Note:** the SDK’s own fallback (`installUrl` / protocol handoff) uses `window.location.href`, which **does** leave the current page. Use `onNeedInstall` when you want to keep the site open.

| OS | Typical package | Notes |
|----|-----------------|--------|
| macOS | `download-tool.dmg` | Open → run **Download Tool.app** |
| Windows | `download-tool-portable.zip` | Unzip → run `download-tool.exe` (keep `aria2c.exe` + CA beside it) |
| Linux | (not packaged yet) | Point at your own tarball/AppImage when you ship one |

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
2. If `size < sizeThreshold` (default **104857600**), download in the browser and return `{ mode: "browser" }`. The SDK `fetch`es the file (CORS) into a Blob, then clicks `<a download>` on an object URL so the filename works **cross-origin**. If the file host does not allow CORS, it falls back to a direct `<a download>` (browsers then ignore `download` and may open text in-tab). Prefer a same-origin proxy URL or `Access-Control-Allow-Origin` on the file server.
3. Otherwise `GET http://127.0.0.1:{localPort}/health` with a **400ms** timeout. The default port is **29128**.
4. Health ready (`ok: true` and `aria2` is not `false`): `POST /v1/tasks` with camelCase `{ url, fileName, size, referrer, userAgent }` and return `{ mode: "local", taskId }`.
5. Connection error, or a health body that is not ready: set `window.location.href` to `{protocol}://add?url=...&fileName=...&size=...` (default scheme `dltool`), then poll `/health` for about **15s** (`config.protocolSettleMs`). This covers the browser’s “Open download-tool.exe?” confirmation and cold start.
6. If that poll sees the app, return `{ mode: "protocol" }`. If the app is still unreachable, call `onNeedInstall` when it was provided. If it was omitted and `installUrl` is set, open that URL. Return `{ mode: "install" }`.
7. A first-probe timeout uses that same install path and does not open the protocol URL.

`referrer` comes from `document.referrer` (or the page URL). `userAgent` comes from `navigator.userAgent`. Empty values are left off the task body and the protocol URL.

## Tests

```bash
cd sdk && npm test
```

Requires Node **18+**. `fetch` and `window` are mocked.
