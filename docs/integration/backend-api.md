# Website backend API

Contract for the site that lists files and hands the JS SDK a short-lived download URL. Adapted from design §5. This repository does not ship the production backend. `mock-backend/` implements the same HTTP shape for local integration.

Download URLs are **HTTP**, not HTTPS. The desktop app and the SDK reject any other scheme. A later move to HTTPS can keep these field names.

The client never sends a server password, a disk root, or an absolute path. Identity is an opaque `fileId`.

## Run the mock

From the repository root:

```bash
node mock-backend/src/server.mjs
```

Listens on `http://127.0.0.1:9090`. No npm install. Node 22.18 or newer.

| Variable | Default | Meaning |
|----------|---------|---------|
| `PORT` | `9090` | Listen port. Ticket URLs use this port. |
| `DOWNLOAD_TICKET_SECRET` | `dev-only-mock-secret` | HMAC key. Local only. Production sets its own and does not use this default. |
| `TICKET_TTL_SECONDS` | `900` | Ticket lifetime. Clamped to **300–1800** seconds (5–30 minutes). |
| `MOCK_DISABLE_RANGE` | unset | `1` ignores `Range` and always returns `200`. Negative check only. |
| `MOCK_X_ACCEL` | unset | `1` returns `X-Accel-Redirect` and an empty body after the ticket verifies. |

The example install page is `examples/install-page/index.html`. The mock also serves it at `http://127.0.0.1:9090/download-tool`.

Logs are one JSON object per response: `fileId`, `status`, `ms`. They do not include file bytes, the HMAC secret, or a disk path.

## 1. File identity

- Callers use an opaque **`fileId`**.
- `GET /api/files` does not return a server absolute path.
- `POST /api/download-ticket` does not accept a filesystem path. An unknown `fileId` is `404`. A directory `fileId` is `400`.
- Responses do not include server accounts, passwords, or a disk root.

## 2. Directory listing

```http
GET /api/files?parentId=d_sets
```

Omit `parentId`, or pass `root`, for the top of the tree.

```json
{
  "items": [
    {
      "fileId": "f_data",
      "name": "data.zip",
      "size": 1234567890,
      "isDir": false,
      "updatedAt": "2026-10-17T12:00:00Z"
    }
  ]
}
```

| Status | When |
|--------|------|
| 200 | `items` for that parent |
| 400 | `parentId` is a file |
| 404 | unknown `parentId` |

Mock tree (no absolute paths):

| fileId | name | size | parent |
|--------|------|------|--------|
| `f_readme` | `readme.txt` | 128 | root |
| `f_sample` | `sample.bin` | 4096 | root |
| `f_data` | `data.zip` | 1234567890 | root |
| `d_sets` | `datasets` | 0 (directory) | root |
| `f_part` | `part-000.bin` | 65536 | `d_sets` |

The SDK sends `size < 104857600` (100 MiB) through the browser. `f_data` (`1234567890`) is the large-file sample. `f_readme` and `f_sample` stay on the browser path.

## 3. Download ticket

```http
POST /api/download-ticket
Content-Type: application/json

{ "fileId": "f_data" }
```

```json
{
  "fileId": "f_data",
  "fileName": "data.zip",
  "size": 1234567890,
  "downloadUrl": "http://127.0.0.1:9090/d/<signed-token>",
  "expiresAt": 1730000000
}
```

`expiresAt` is Unix seconds.

| Status | When |
|--------|------|
| 200 | ticket issued |
| 400 | missing `fileId`, invalid JSON, or the id is a directory |
| 404 | unknown `fileId` |

Rules:

- `downloadUrl` is `http://` and short-lived. Default **15 minutes**. Configure **5–30 minutes**.
- The bytes endpoint checks the HMAC and `expiresAt`.
- Expired ticket: **410**. Bad signature or malformed token: **403**.
- The bytes response supports **`Range`** and **`206 Partial Content`**. aria2 uses both for multiple connections and resume.
- The ticket does not carry a server account, password, or disk path.

Token format (mock and the recommended production shape):

```text
base64url(JSON { "fileId", "expiresAt" }) + "." + base64url(HMAC-SHA256(secret, payload))
```

Compare the signature with a constant-time check. Trust `fileId` from the token only after that check.

## 4. Bytes: `GET /d/:token`

Success with no `Range`:

```http
HTTP/1.1 200 OK
Accept-Ranges: bytes
Content-Length: 1234567890
Content-Type: application/octet-stream
Content-Disposition: attachment; filename="data.zip"
```

A single range:

```http
GET /d/<token>
Range: bytes=1000000-1000015

HTTP/1.1 206 Partial Content
Accept-Ranges: bytes
Content-Range: bytes 1000000-1000015/1234567890
Content-Length: 16
```

Also accept `bytes=start-` and `bytes=-suffix`. An unsatisfiable range is **416** with `Content-Range: bytes */<size>`.

The mock generates bytes while it writes them, in **64KB** chunks. It does not read the whole object into memory. Production should do the same when the application process is the one writing the body.

`HEAD /d/:token` returns the same headers and an empty body.

### Large-file egress

Do not read an entire file into memory and then write it out.

Preferred path:

1. The application, or the gateway, only authenticates and verifies the ticket.
2. Hand the body to the web server with Nginx `X-Accel-Redirect` (or the Caddy / equivalent `sendfile` offload).
3. Set proxy and send timeouts long enough for the transfer.
4. Cap per-IP concurrency, bandwidth, and the global rate when you need to.
5. Log `fileId`, status, and duration. Do not log file contents.

Nginx sketch. The redirect value is an **internal URI**, not a disk path. The disk path stays in the server config. `internal` stops clients from requesting that location themselves.

```nginx
location /internal/ {
    internal;
    alias /var/lib/download-tool/files/;
    sendfile on;
}

location /d/ {
    # Application that verifies the ticket, then returns X-Accel-Redirect.
    proxy_pass http://127.0.0.1:8080;
    proxy_read_timeout 3600s;
    send_timeout 3600s;
    proxy_buffering off;
}
```

With `MOCK_X_ACCEL=1` the mock answers a valid ticket like this and does not write file bytes:

```http
HTTP/1.1 200 OK
X-Accel-Redirect: /internal/f_data
Content-Length: 0
```

Map `/internal/<fileId>` to the real file only inside that trusted config. If the application must write the body itself, stream **64KB–1MB** chunks and implement `Range` as above.

`MOCK_DISABLE_RANGE=1` is a negative fixture: `Range` is ignored and the response is `200` for the whole object. aria2 multi-connection and resume will not work against that mode. Leave it unset in normal use.

## 5. Site config for the SDK

The page passes this to the SDK. It is not a download credential.

```json
{
  "sizeThreshold": 104857600,
  "installUrl": "http://127.0.0.1:9090/download-tool",
  "localPort": 29128,
  "protocol": "dltool"
}
```

- `sizeThreshold`: default **100MB** (`104857600`). `size < sizeThreshold` downloads in the browser. Larger files go to the local app.
- `localPort`: the desktop app’s fixed loopback port.
- `installUrl`: the install guide. The example page has placeholder links for Windows, macOS, and Linux.
- `protocol`: `dltool`.

```ts
await download({
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

SDK order, for the checklist below:

1. `POST {apiBase}/api/download-ticket` with `{ fileId }`. Keep `downloadUrl` as the `http://` value from the ticket.
2. If `size < sizeThreshold`, the browser downloads that URL.
3. Otherwise `GET http://127.0.0.1:29128/health` (400ms). Ready means HTTP success, `ok: true`, and `aria2` is not `false`.
4. Health ready: `POST /v1/tasks` with camelCase `{ url, fileName, size, referrer, userAgent }`.
5. Connection refused, or health that is not ready: `dltool://add?url=...&fileName=...&size=...`, then poll `/health` for about 2s.
6. If the app answers that poll, the handoff stays on the protocol. If it is still unreachable, `onNeedInstall` or `installUrl`. A first-probe timeout uses that install path directly.

The local API binds `127.0.0.1` only. `GET`/`POST`/`OPTIONS` on `/health`, `/v1/tasks`, and `/v1/invoke` echo the request `Origin` (or `*` when there is none) and send `Access-Control-Allow-Private-Network: true`. The mock allows the request origin on its own routes (and `*` when there is no `Origin`).

## Manual integration checklist

Mock only, from the repository root. Another terminal holds the server.

- [ ] `node mock-backend/src/server.mjs` prints `mock backend http://127.0.0.1:9090`.
- [ ] `curl -sS http://127.0.0.1:9090/api/files` returns `fileId` values. The body has no `/Users/`, `/var/`, or `password`.
- [ ] `curl -sS 'http://127.0.0.1:9090/api/files?parentId=d_sets'` returns `f_part`. `parentId=missing` is 404.
- [ ] Ticket for the large file. `downloadUrl` starts with `http://127.0.0.1:9090/d/` and does not start with `https://`.

  ```bash
  curl -sS -X POST http://127.0.0.1:9090/api/download-ticket \
    -H 'content-type: application/json' \
    -d '{"fileId":"f_data"}'
  ```

- [ ] `POST` with `{"fileId":"d_sets"}` is 400. `{"fileId":"/etc/passwd"}` is 404 and does not read that path.
- [ ] No `Range`: `curl -sS -D - -o /dev/null "$URL"` is **200**, `Accept-Ranges: bytes`, `Content-Length` equals `size`. Use `f_readme` (128 bytes), not `f_data`.
- [ ] `curl -sS -D - -o /tmp/slice.bin -H 'Range: bytes=0-15' "$URL"` is **206**, `Content-Range: bytes 0-15/<size>`, and the file is 16 bytes.
- [ ] A range past the end is **416** with `Content-Range: bytes */<size>`.
- [ ] Change one character of the signature. `GET` is **403**.
- [ ] Expired ticket is **410**:

  ```bash
  TOKEN=$(node --input-type=module -e '
  import { signToken } from "./mock-backend/src/server.mjs";
  process.stdout.write(signToken("f_readme", 1, process.env.DOWNLOAD_TICKET_SECRET || "dev-only-mock-secret"));
  ')
  curl -sS -D - -o /dev/null "http://127.0.0.1:9090/d/$TOKEN"
  ```

- [ ] Restart with `MOCK_DISABLE_RANGE=1`. The same `Range` request is **200** and the body is the whole small file. Stop that process before the next check.
- [ ] Restart with `MOCK_X_ACCEL=1`. A valid `GET /d/:token` is **200**, `X-Accel-Redirect: /internal/f_readme`, empty body. The header is not a disk path.
- [ ] Open `http://127.0.0.1:9090/download-tool`. Windows, macOS, and Linux links are `http://127.0.0.1:9090/releases/...` placeholders (those paths are 404 text until you publish installers).
- [ ] SDK against `apiBase: "http://127.0.0.1:9090"`: `f_readme` / size 128 returns `{ mode: "browser" }` and uses the ticket URL. `f_data` / size 1234567890 probes `http://127.0.0.1:29128/health`, then posts `/v1/tasks` or opens `dltool://add?...`.
- [ ] With the desktop app running, the queued task downloads the **http** ticket. aria2 sends `Range`. A 410 on an old ticket fails the task; the user clicks Download on the site again.
- [ ] Server log lines contain `fileId`, `status`, and `ms` only.

Production pass, after the mock checks:

- [ ] Listing and ticket responses still have no absolute path and no server password.
- [ ] `downloadUrl` is `http://` and stops working after the configured TTL.
- [ ] `Range` / `206` works on the real object, including a resume from the middle.
- [ ] The process does not hold the whole file in memory. Offload with `X-Accel-Redirect` (or stream 64KB–1MB).
- [ ] Proxy and send timeouts cover the largest file you publish.
- [ ] Install page links are the real Windows, macOS, and Linux packages, still advertised to the SDK as `installUrl`.
