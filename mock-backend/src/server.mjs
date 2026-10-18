// Run: node mock-backend/src/server.mjs
import { createHmac, timingSafeEqual } from "node:crypto";
import { readFile } from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

const DEFAULT_PORT = 9090;
const DEFAULT_SECRET = "dev-only-mock-secret";
const DEFAULT_TTL_SECONDS = 900;
const MIN_TTL_SECONDS = 300;
const MAX_TTL_SECONDS = 1800;
const CHUNK_BYTES = 64 * 1024;
const ROOT_ID = "root";

const here = path.dirname(fileURLToPath(import.meta.url));
const defaultInstallPage = path.resolve(here, "../../examples/install-page/index.html");

/** @type {readonly { fileId: string, name: string, size: number, isDir: boolean, parentId: string, updatedAt: string }[]} */
const FILES = [
  {
    fileId: "f_readme",
    name: "readme.txt",
    size: 128,
    isDir: false,
    parentId: ROOT_ID,
    updatedAt: "2026-10-17T12:00:00Z",
  },
  {
    fileId: "f_sample",
    name: "sample.bin",
    size: 4096,
    isDir: false,
    parentId: ROOT_ID,
    updatedAt: "2026-10-17T12:00:00Z",
  },
  {
    fileId: "f_data",
    name: "data.zip",
    size: 1234567890,
    isDir: false,
    parentId: ROOT_ID,
    updatedAt: "2026-10-17T12:00:00Z",
  },
  {
    fileId: "d_sets",
    name: "datasets",
    size: 0,
    isDir: true,
    parentId: ROOT_ID,
    updatedAt: "2026-10-17T12:00:00Z",
  },
  {
    fileId: "f_part",
    name: "part-000.bin",
    size: 65536,
    isDir: false,
    parentId: "d_sets",
    updatedAt: "2026-10-17T12:05:00Z",
  },
];

/**
 * @typedef {object} MockOptions
 * @property {string} [secret]
 * @property {number | string} [ttlSeconds]
 * @property {boolean} [honorRange]
 * @property {boolean} [xAccel]
 * @property {string} [installPagePath]
 */

/**
 * @param {string} fileId
 * @param {number} expiresAt Unix seconds.
 * @param {string} secret
 */
export function signToken(fileId, expiresAt, secret) {
  const payload = Buffer.from(JSON.stringify({ fileId, expiresAt }), "utf8").toString("base64url");
  const signature = createHmac("sha256", secret).update(payload).digest("base64url");
  return `${payload}.${signature}`;
}

/**
 * @param {MockOptions} [options]
 */
export function createServer(options = {}) {
  const secret = options.secret || DEFAULT_SECRET;
  const ttlSeconds = clampTtl(options.ttlSeconds);
  const honorRange = options.honorRange !== false;
  const xAccel = options.xAccel === true;
  const installPage = options.installPagePath || defaultInstallPage;

  return http.createServer((req, res) => {
    const started = Date.now();
    let fileId = null;
    applyCors(req, res);
    res.on("finish", () => {
      console.log(JSON.stringify({ fileId, status: res.statusCode, ms: Date.now() - started }));
    });
    handle(req, res, {
      secret,
      ttlSeconds,
      honorRange,
      xAccel,
      installPage,
      setFileId(id) {
        fileId = id;
      },
    }).catch((error) => {
      if (res.headersSent || res.writableEnded) {
        res.destroy();
        return;
      }
      const status = typeof error.status === "number" ? error.status : 500;
      sendJson(res, status, { error: status === 500 ? "internal error" : error.message });
    });
  });
}

/**
 * @param {number | string | undefined} value
 */
function clampTtl(value) {
  if (value === undefined || value === null || value === "") return DEFAULT_TTL_SECONDS;
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return DEFAULT_TTL_SECONDS;
  return Math.min(MAX_TTL_SECONDS, Math.max(MIN_TTL_SECONDS, Math.floor(parsed)));
}

/**
 * @param {import("node:http").IncomingMessage} req
 * @param {import("node:http").ServerResponse} res
 */
function applyCors(req, res) {
  const origin = req.headers.origin;
  if (typeof origin === "string" && origin.length > 0) {
    res.setHeader("access-control-allow-origin", origin);
    res.setHeader("vary", "Origin");
    res.setHeader("access-control-allow-credentials", "true");
  } else {
    res.setHeader("access-control-allow-origin", "*");
  }
  res.setHeader("access-control-allow-methods", "GET, POST, HEAD, OPTIONS");
  res.setHeader("access-control-allow-headers", "Content-Type, Range, Accept");
  res.setHeader(
    "access-control-expose-headers",
    "Accept-Ranges, Content-Range, Content-Length, Content-Disposition",
  );
}

/**
 * @param {import("node:http").IncomingMessage} req
 * @param {import("node:http").ServerResponse} res
 * @param {{ secret: string, ttlSeconds: number, honorRange: boolean, xAccel: boolean, installPage: string, setFileId: (id: string) => void }} ctx
 */
async function handle(req, res, ctx) {
  if (req.method === "OPTIONS") {
    res.writeHead(204);
    res.end();
    return;
  }

  const url = new URL(req.url || "/", "http://127.0.0.1");
  if (req.method === "GET" && url.pathname === "/api/files") {
    listFiles(url, res, ctx);
    return;
  }
  if (url.pathname === "/api/download-ticket") {
    if (req.method !== "POST") {
      sendJson(res, 405, { error: "method not allowed" });
      return;
    }
    await issueTicket(req, res, ctx);
    return;
  }
  if (url.pathname.startsWith("/d/")) {
    if (req.method !== "GET" && req.method !== "HEAD") {
      sendJson(res, 405, { error: "method not allowed" });
      return;
    }
    await sendDownload(req, res, url.pathname.slice(3), ctx);
    return;
  }
  if (req.method === "GET" && (url.pathname === "/download-tool" || url.pathname === "/download-tool/")) {
    await sendInstallPage(res, ctx.installPage);
    return;
  }
  if (req.method === "GET" && url.pathname.startsWith("/releases/")) {
    sendText(res, 404, "Placeholder. Replace this URL with the built installer.\n");
    return;
  }
  if (req.method === "GET" && url.pathname === "/") {
    sendText(
      res,
      200,
      "mock backend\nGET /api/files\nPOST /api/download-ticket\nGET /d/:token\nGET /download-tool\n",
    );
    return;
  }
  sendJson(res, 404, { error: "not found" });
}

/**
 * @param {URL} url
 * @param {import("node:http").ServerResponse} res
 * @param {{ setFileId: (id: string) => void }} ctx
 */
function listFiles(url, res, ctx) {
  const parentId = url.searchParams.get("parentId") || ROOT_ID;
  ctx.setFileId(parentId);
  if (parentId !== ROOT_ID) {
    const parent = FILES.find((item) => item.fileId === parentId);
    if (!parent) {
      sendJson(res, 404, { error: "unknown parent" });
      return;
    }
    if (!parent.isDir) {
      sendJson(res, 400, { error: "parentId is not a directory" });
      return;
    }
  }
  const items = FILES.filter((item) => item.parentId === parentId).map(publicItem);
  sendJson(res, 200, { items });
}

/**
 * @param {{ fileId: string, name: string, size: number, isDir: boolean, updatedAt: string }} item
 */
function publicItem(item) {
  return {
    fileId: item.fileId,
    name: item.name,
    size: item.size,
    isDir: item.isDir,
    updatedAt: item.updatedAt,
  };
}

/**
 * @param {import("node:http").IncomingMessage} req
 * @param {import("node:http").ServerResponse} res
 * @param {{ secret: string, ttlSeconds: number, setFileId: (id: string) => void }} ctx
 */
async function issueTicket(req, res, ctx) {
  const raw = await readBody(req);
  let body;
  try {
    body = JSON.parse(raw || "{}");
  } catch {
    sendJson(res, 400, { error: "invalid json" });
    return;
  }
  if (!body || typeof body.fileId !== "string" || body.fileId.length === 0) {
    sendJson(res, 400, { error: "fileId is required" });
    return;
  }
  ctx.setFileId(body.fileId);
  const file = FILES.find((item) => item.fileId === body.fileId);
  if (!file) {
    sendJson(res, 404, { error: "unknown file" });
    return;
  }
  if (file.isDir) {
    sendJson(res, 400, { error: "fileId is a directory" });
    return;
  }
  const expiresAt = Math.floor(Date.now() / 1000) + ctx.ttlSeconds;
  const token = signToken(file.fileId, expiresAt, ctx.secret);
  const address = req.socket.server.address();
  const port = address && typeof address === "object" ? address.port : DEFAULT_PORT;
  sendJson(res, 200, {
    fileId: file.fileId,
    fileName: file.name,
    size: file.size,
    downloadUrl: `http://127.0.0.1:${port}/d/${token}`,
    expiresAt,
  });
}

/**
 * @param {import("node:http").IncomingMessage} req
 * @param {import("node:http").ServerResponse} res
 * @param {string} token
 * @param {{ secret: string, honorRange: boolean, xAccel: boolean, setFileId: (id: string) => void }} ctx
 */
async function sendDownload(req, res, token, ctx) {
  let decoded = token;
  try {
    decoded = decodeURIComponent(token);
  } catch {
    sendJson(res, 403, { error: "invalid ticket" });
    return;
  }
  const verified = verifyToken(decoded, ctx.secret);
  if (verified.fileId) ctx.setFileId(verified.fileId);
  if (verified.error) {
    sendJson(res, verified.error, { error: verified.error === 410 ? "ticket expired" : "invalid ticket" });
    return;
  }
  const file = FILES.find((item) => item.fileId === verified.fileId && !item.isDir);
  if (!file) {
    sendJson(res, 404, { error: "unknown file" });
    return;
  }
  if (ctx.xAccel) {
    res.writeHead(200, {
      "x-accel-redirect": `/internal/${file.fileId}`,
      "content-length": "0",
      "cache-control": "no-store",
    });
    res.end();
    return;
  }

  const rangeHeader = ctx.honorRange ? headerValue(req.headers.range) : undefined;
  const range = rangeHeader ? parseRange(rangeHeader, file.size) : null;
  if (range?.error) {
    res.writeHead(416, {
      "content-range": `bytes */${file.size}`,
      "accept-ranges": "bytes",
      "content-length": "0",
    });
    res.end();
    return;
  }

  const start = range ? range.start : 0;
  const end = range ? range.end : file.size - 1;
  const status = range ? 206 : 200;
  /** @type {Record<string, string>} */
  const headers = {
    "content-type": "application/octet-stream",
    "content-length": String(end - start + 1),
    "accept-ranges": "bytes",
    "content-disposition": `attachment; filename="${file.name}"`,
    "cache-control": "no-store",
  };
  if (range) headers["content-range"] = `bytes ${start}-${end}/${file.size}`;
  res.writeHead(status, headers);
  if (req.method === "HEAD") {
    res.end();
    return;
  }
  streamBytes(res, start, end, seedFor(file.fileId));
}

/**
 * @param {string} token
 * @param {string} secret
 * @returns {{ error?: number, fileId?: string }}
 */
function verifyToken(token, secret) {
  const dot = token.indexOf(".");
  if (dot <= 0 || token.indexOf(".", dot + 1) !== -1) return { error: 403 };
  const payload = token.slice(0, dot);
  const signature = token.slice(dot + 1);
  const expected = createHmac("sha256", secret).update(payload).digest("base64url");
  const actualBuffer = Buffer.from(signature);
  const expectedBuffer = Buffer.from(expected);
  if (actualBuffer.length !== expectedBuffer.length || !timingSafeEqual(actualBuffer, expectedBuffer)) {
    return { error: 403 };
  }
  let parsed;
  try {
    parsed = JSON.parse(Buffer.from(payload, "base64url").toString("utf8"));
  } catch {
    return { error: 403 };
  }
  if (!parsed || typeof parsed.fileId !== "string" || typeof parsed.expiresAt !== "number") {
    return { error: 403 };
  }
  if (parsed.expiresAt <= Math.floor(Date.now() / 1000)) return { error: 410, fileId: parsed.fileId };
  return { fileId: parsed.fileId };
}

/**
 * @param {string} header
 * @param {number} size
 * @returns {{ start: number, end: number } | { error: true }}
 */
function parseRange(header, size) {
  const match = /^bytes=(\d*)-(\d*)$/.exec(header.trim());
  if (!match || size <= 0) return { error: true };
  const startText = match[1];
  const endText = match[2];
  if (startText === "" && endText === "") return { error: true };
  if (startText === "") {
    const suffix = Number(endText);
    if (!Number.isInteger(suffix) || suffix <= 0) return { error: true };
    const length = Math.min(suffix, size);
    return { start: size - length, end: size - 1 };
  }
  const start = Number(startText);
  if (!Number.isInteger(start) || start < 0 || start >= size) return { error: true };
  const end = endText === "" ? size - 1 : Number(endText);
  if (!Number.isInteger(end) || end < start) return { error: true };
  return { start, end: Math.min(end, size - 1) };
}

/**
 * @param {import("node:http").ServerResponse} res
 * @param {number} start
 * @param {number} end
 * @param {number} seed
 */
function streamBytes(res, start, end, seed) {
  let offset = start;
  let stopped = false;
  res.on("close", () => {
    if (!res.writableEnded) stopped = true;
  });

  const pump = () => {
    while (!stopped && offset <= end) {
      const length = Math.min(CHUNK_BYTES, end - offset + 1);
      const buffer = Buffer.allocUnsafe(length);
      for (let index = 0; index < length; index += 1) {
        buffer[index] = (offset + index + seed) & 0xff;
      }
      offset += length;
      if (!res.write(buffer)) {
        res.once("drain", pump);
        return;
      }
    }
    if (!stopped) res.end();
  };
  pump();
}

/**
 * @param {string} fileId
 */
function seedFor(fileId) {
  let seed = 0;
  for (const char of fileId) seed = (Math.imul(seed, 33) + char.charCodeAt(0)) >>> 0;
  return seed || 1;
}

/**
 * @param {import("node:http").ServerResponse} res
 * @param {string} installPage
 */
async function sendInstallPage(res, installPage) {
  try {
    const html = await readFile(installPage);
    res.writeHead(200, {
      "content-type": "text/html; charset=utf-8",
      "content-length": html.length,
      "cache-control": "no-store",
    });
    res.end(html);
  } catch {
    sendJson(res, 404, { error: "install page not found" });
  }
}

/**
 * @param {import("node:http").IncomingMessage} req
 */
function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    req.on("data", (chunk) => {
      size += chunk.length;
      if (size > 1_000_000) {
        reject(Object.assign(new Error("body too large"), { status: 413 }));
        req.destroy();
        return;
      }
      chunks.push(chunk);
    });
    req.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    req.on("error", reject);
  });
}

/**
 * @param {string | string[] | undefined} value
 */
function headerValue(value) {
  return Array.isArray(value) ? value[0] : value;
}

/**
 * @param {import("node:http").ServerResponse} res
 * @param {number} status
 * @param {unknown} body
 */
function sendJson(res, status, body) {
  const payload = JSON.stringify(body);
  res.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "content-length": Buffer.byteLength(payload),
    "cache-control": "no-store",
  });
  res.end(payload);
}

/**
 * @param {import("node:http").ServerResponse} res
 * @param {number} status
 * @param {string} body
 */
function sendText(res, status, body) {
  res.writeHead(status, {
    "content-type": "text/plain; charset=utf-8",
    "content-length": Buffer.byteLength(body),
    "cache-control": "no-store",
  });
  res.end(body);
}

const isMain =
  process.argv[1] !== undefined && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isMain) {
  const port = Number(process.env.PORT) || DEFAULT_PORT;
  const server = createServer({
    secret: process.env.DOWNLOAD_TICKET_SECRET || DEFAULT_SECRET,
    ttlSeconds: process.env.TICKET_TTL_SECONDS,
    honorRange: process.env.MOCK_DISABLE_RANGE !== "1",
    xAccel: process.env.MOCK_X_ACCEL === "1",
  });
  server.listen(port, "127.0.0.1", () => {
    const address = server.address();
    const bound = address && typeof address === "object" ? address.port : port;
    console.log(`mock backend http://127.0.0.1:${bound}`);
  });
}
