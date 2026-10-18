# Download Tool

Desktop downloader for large HTTP files. The app is Rust + egui, with aria2 doing the transfer. A website opens:

```text
dltool://add?url=<percent-encoded-http-url>&fileName=...&size=...
```

The inner download URL must be `http://`. `fileName`, `size`, `referrer`, and `userAgent` are optional. A cold start parses those arguments and enqueues the download. If the app is already running, the new process forwards the same arguments and exits.

The local API listens on **`127.0.0.1:29128`** only. JSON on that API and in the JS SDK is camelCase (`fileName`, `taskId`).

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

`POST` returns `{"taskId":"..."}`. `GET /v1/tasks` lists the queue. Only `http://` URLs are accepted.

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

[`sdk/`](sdk/) is the browser helper. Small files download in the browser. Files at or above the threshold (default **104857600**, 100MB) probe the local app, then post a task or open `dltool://`.

```bash
cd sdk && npm test
```

Node 22.18 or newer. Usage, field names, and the probe order are in [sdk/README.md](sdk/README.md).

```ts
import { download } from "download-tool-sdk";

const result = await download({
  fileId: "f_data",
  size: 1234567890,
  apiBase: "http://127.0.0.1:9090",
  config: {
    sizeThreshold: 104857600,
    installUrl: "http://127.0.0.1:9090/download-tool",
    localPort: 29128,
    protocol: "dltool",
  },
});
```

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

Installer artifacts for Windows, macOS, and Linux are not produced yet. Use the run and smoke steps above until that milestone.

When installers are added, use [cargo-packager](https://github.com/crabnebula-dev/cargo-packager) on the `download-app` package:

```bash
cargo install cargo-packager --locked
```

Target formats: NSIS on Windows, a `.app` plus `.dmg` on macOS, and `.deb` on Linux. The macOS bundle `Info.plist` must include the `CFBundleURLTypes` block above so `dltool://` can be registered. Leave the packager updater disabled; this version has no in-app update.
