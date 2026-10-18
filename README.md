# Download Tool

Desktop downloader for large HTTP(S) files. The app is Rust + egui; aria2 does the transfer. Websites hand off big files via the **[dltool-sdk](https://www.npmjs.com/package/dltool-sdk)** package or the `dltool://` protocol:

```text
dltool://add?url=<percent-encoded-http-url>&fileName=...&size=...
```

The inner download URL must be `http://` or `https://`. `fileName`, `size`, `referrer`, and `userAgent` are optional. A cold start parses those arguments and enqueues the download. If the app is already running, the new process forwards the same arguments and exits.

The local API listens on **`127.0.0.1:29128`** only. JSON on that API and in the JS SDK is camelCase (`fileName`, `taskId`). Task history is stored in SQLite under the OS app-support directory (`tasks.db`).
## Build

Rust edition 2021. From the repository root:

```bash
cargo build -p download-app
cargo test
```

The binary is `download-tool` (`target/debug/download-tool`, or `target/release/download-tool` after `cargo build --release -p download-app`).

Workspace crates: `download-core` (tasks and aria2 RPC), `download-local-api` (the loopback API), `download-app` (window, tray, single instance, `dltool://`).

On Linux the tray needs the system packages named by `tray-icon`: gtk3, xdotool, and libappindicator or libayatana-appindicator.

## Vendor aria2

`download-core` looks for `aria2c` in `DOWNLOAD_TOOL_ARIA2C`, then `vendor/aria2/<rust-target-triple>/aria2c` (`aria2c.exe` on Windows). Layout, official builds, and the CI note are in [vendor/aria2/README.md](vendor/aria2/README.md).

Do not commit the binaries. For a machine that already has aria2:

```bash
export DOWNLOAD_TOOL_ARIA2C="$(command -v aria2c)"
```

Without a binary the window still opens and tasks can be queued. Downloads do not start until `aria2c` is available. The RPC secret is a per-run UUID and is not exposed on the local task API.

HTTPS downloads need a CA bundle (aria2 OpenSSL builds do not use the OS trust store by themselves). The app passes `--ca-certificate` from, in order:

1. `DOWNLOAD_TOOL_CA_FILE`
2. `SSL_CERT_FILE` / `CURL_CA_BUNDLE`
3. `curl-ca-bundle.crt` / `cacert.pem` next to `aria2c` or this app
4. macOS/Linux: `/etc/ssl/cert.pem` (and similar)
5. Windows: Git for Windows `...\Git\mingw64\ssl\certs\ca-bundle.crt` if installed

**Windows tip:** put [cacert.pem](https://curl.se/ca/cacert.pem) (or `curl-ca-bundle.crt`) beside `aria2c.exe`, or:

```bat
set DOWNLOAD_TOOL_CA_FILE=C:\path\to\cacert.pem
```

## Run

```bash
cargo run -p download-app
```

Startup registers `dltool://` (best effort; a failure is printed and the app still opens) and binds `http://127.0.0.1:29128`.

```http
GET /health
{"ok":true,"aria2":true,"version":"0.1.0"}
```

`/health` is **503** with `"ok": false` and `"aria2": false` when aria2 is not ready. The SDK does not treat that as a local handoff. Responses include CORS and `Access-Control-Allow-Private-Network` so a page on another origin can probe the loopback API.

```http
POST /v1/tasks
Content-Type: application/json

{"url":"http://127.0.0.1:9090/d/<token>","fileName":"data.zip","size":1234567890}
```

`POST` returns `{"taskId":"..."}`. `GET /v1/tasks` lists the queue. Only `http://` and `https://` URLs are accepted.

Pass a protocol URL on the command line when the OS will not launch the bare `cargo run` binary (see macOS below):

```bash
cargo run -p download-app -- 'dltool://add?url=http%3A%2F%2F127.0.0.1%2Ffile.bin&fileName=file.bin'
```

## Local smoke

Start the app in one terminal, then from the repository root:

```bash
./scripts/smoke-local.sh
```

The script `GET`s `/health` and `POST`s `/v1/tasks` against `http://127.0.0.1:29128`. It does not launch the app. The posted URL is a loopback placeholder (`http://127.0.0.1:9/smoke-local.bin`); the same URL returns the existing `taskId` on a later run. `DOWNLOAD_TOOL_URL` overrides the base URL.

This `cargo run` plus protocol registration is the first smoke. Platform installers are a later step (below).

## SDK

Published on npm as **[`dltool-sdk`](https://www.npmjs.com/package/dltool-sdk)** (npm blocks the word `download` in package names). Source is in [`sdk/`](sdk/).

Small files download in the browser. Files at or above the threshold (default **104857600**, 100MB) probe the local app, then `POST /v1/tasks` or open `dltool://`. Full probe order and options: [sdk/README.md](sdk/README.md).

```bash
npm install dltool-sdk
```

```ts
import { download } from "dltool-sdk";

// Direct URL (no ticket API):
await download({
  downloadUrl: "https://cdn.example.com/file.zip",
  fileName: "file.zip",
  size: 1234567890,
  config: {
    sizeThreshold: 104857600,
    installUrl: "https://your-host/download-tool",
    localPort: 29128,
  },
});

// Or resolve via website ticket API:
await download({
  fileId: "f_data",
  size: 1234567890,
  apiBase: "http://127.0.0.1:9090",
  config: {
    installUrl: "http://127.0.0.1:9090/download-tool",
    localPort: 29128,
  },
});
```

CDN / `<script>`:

```html
<script src="https://unpkg.com/dltool-sdk/dist/dltool-sdk.iife.js"></script>
<script>
  DownloadToolSdk.download({ /* same options */ });
</script>
```

Local build / test:

```bash
cd sdk && npm install && npm run build && npm test
```

Node **18+**. Bundlers get ESM/CJS from `dist/`; the IIFE global is `DownloadToolSdk` (alias `DownloadTool`).
## Mock backend

The mock implements the website ticket API on `http://127.0.0.1:9090` (file list, HMAC ticket, `Range` / `206`). No npm install.

```bash
node mock-backend/src/server.mjs
```

`cd mock-backend && npm test` runs the same server tests. The contract, `X-Accel-Redirect` notes, and the manual checklist are in [docs/integration/backend-api.md](docs/integration/backend-api.md). The example install page is [examples/install-page/index.html](examples/install-page/index.html), also served at `http://127.0.0.1:9090/download-tool`.

## Protocol registration

Startup tries to register the `dltool` scheme. Failure is printed and the app still opens.

### Windows

The current user hive:

- `HKCU\Software\Classes\dltool` — default `URL:Download Tool Protocol`, and an empty `URL Protocol` value
- `HKCU\Software\Classes\dltool\shell\open\command` — `"<path-to-download-tool>" "%1"`

### Linux

Writes `dltool.desktop` to `$XDG_DATA_HOME/applications` (or `~/.local/share/applications`) with `MimeType=x-scheme-handler/dltool;`, then runs:

```bash
xdg-mime default dltool.desktop x-scheme-handler/dltool
```

### macOS

Launch Services associates a scheme only with an app bundle. Put this in the bundle `Info.plist`:

```xml
<key>CFBundleURLTypes</key>
<array>
    <dict>
        <key>CFBundleURLName</key>
        <string>Download Tool</string>
        <key>CFBundleURLSchemes</key>
        <array>
            <string>dltool</string>
        </array>
    </dict>
</array>
```

Refresh Launch Services after the bundle is in place (a bundled build does this itself when `Info.plist` already lists `dltool`):

```bash
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f /path/to/Download\ Tool.app
```

`cargo run` is a bare executable, not a `.app`. macOS will not open `dltool://` links from that binary. Pass the URL on the command line instead:

```bash
cargo run -p download-app -- 'dltool://add?url=http%3A%2F%2F127.0.0.1%2Ffile.bin&fileName=file.bin'
```

## Packaging

**Portable first (no installer wizard).** Users unzip / open the app and run it.

### macOS

```bash
./scripts/package-macos.sh
```

Produces `dist/macos/Download Tool.app` and `download-tool.dmg`. Opening the `.app` is enough; dragging to Applications is optional. The DMG includes an Applications shortcut and `README.txt`.

### Windows

On a Windows machine (Rust + `aria2c.exe` on `PATH`):

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-windows-portable.ps1
```

Produces `dist/windows/download-tool/` and `download-tool-portable.zip`. Unzip anywhere and double-click `download-tool.exe` — no Setup.exe. Keep `aria2c.exe` and the CA bundle beside the exe.

### Linux

Not packaged yet; `cargo run -p download-app` / a self-contained folder with `aria2c` beside the binary is the intended portable shape.
