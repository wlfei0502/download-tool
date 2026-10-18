# download-tool SDK

Browser helper that sends small files through a normal download and large files to the desktop app.

```ts
import { download } from "download-tool-sdk";

const result = await download({
  fileId: "f_01hxyz",
  size: 1234567890,
  apiBase: "http://your-host",
  config: {
    sizeThreshold: 104857600,
    installUrl: "http://your-host/download-tool",
    localPort: 29128,
    protocol: "dltool",
  },
  onNeedInstall() {
    // Show the site's own install prompt.
  },
});
```

`DownloadTool.download` is the same function.

## Order

1. `POST {apiBase}/api/download-ticket` with `{ fileId }`. The ticket's `downloadUrl` stays `http://` and is passed through unchanged.
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

Node 22.18 or newer runs the TypeScript tests directly. `fetch` and `window` are mocked.
