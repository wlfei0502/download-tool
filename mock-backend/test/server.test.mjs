import assert from "node:assert/strict";
import { once } from "node:events";
import http from "node:http";
import test from "node:test";

import { createServer, signToken } from "../src/server.mjs";

const SECRET = "test-secret";

/**
 * @param {import("node:http").Server} server
 */
function listen(server) {
  server.listen(0, "127.0.0.1");
  return once(server, "listening");
}

/**
 * @param {import("node:http").Server} server
 * @param {{ method?: string, path: string, headers?: Record<string, string>, body?: string }} request
 */
function request(server, { method = "GET", path, headers = {}, body }) {
  const address = server.address();
  if (!address || typeof address === "string") {
    throw new Error("server is not listening");
  }
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        hostname: "127.0.0.1",
        port: address.port,
        method,
        path,
        headers: { connection: "close", ...headers },
      },
      (res) => {
        const chunks = [];
        res.on("data", (chunk) => chunks.push(chunk));
        res.on("end", () => {
          resolve({
            status: res.statusCode ?? 0,
            headers: res.headers,
            body: Buffer.concat(chunks),
          });
        });
      },
    );
    req.on("error", reject);
    if (body) req.write(body);
    req.end();
  });
}

/**
 * @param {import("../src/server.mjs").MockOptions} [options]
 */
async function start(options = {}) {
  const server = createServer({ secret: SECRET, ...options });
  await listen(server);
  return server;
}

test("lists opaque fileIds and hides absolute paths", async () => {
  const server = await start();
  try {
    const response = await request(server, { path: "/api/files" });
    assert.equal(response.status, 200);
    const payload = JSON.parse(response.body.toString());
    assert.ok(Array.isArray(payload.items));
    assert.ok(payload.items.length >= 2);
    const serialized = JSON.stringify(payload);
    assert.doesNotMatch(serialized, /\/Users\/|\/home\/|\/var\/|[A-Za-z]:\\/);
    assert.doesNotMatch(serialized, /password|secret|absolutePath/i);
    for (const item of payload.items) {
      assert.equal(typeof item.fileId, "string");
      assert.ok(item.fileId.length > 0);
      assert.equal(typeof item.name, "string");
      assert.equal(typeof item.isDir, "boolean");
      assert.equal(typeof item.size, "number");
      assert.equal(typeof item.updatedAt, "string");
      assert.equal("path" in item, false);
    }
    const folder = payload.items.find((item) => item.isDir);
    assert.ok(folder);
    const child = await request(server, { path: `/api/files?parentId=${folder.fileId}` });
    assert.equal(child.status, 200);
    const children = JSON.parse(child.body.toString());
    assert.ok(children.items.every((item) => item.isDir === false));
    assert.equal((await request(server, { path: "/api/files?parentId=missing" })).status, 404);
  } finally {
    server.close();
  }
});

test("issues an http download ticket on 127.0.0.1", async () => {
  const server = await start({ ttlSeconds: 900 });
  try {
    const address = server.address();
    if (!address || typeof address === "string") throw new Error("not listening");
    const response = await request(server, {
      method: "POST",
      path: "/api/download-ticket",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ fileId: "f_data" }),
    });
    assert.equal(response.status, 200);
    const ticket = JSON.parse(response.body.toString());
    assert.equal(ticket.fileId, "f_data");
    assert.equal(ticket.fileName, "data.zip");
    assert.equal(ticket.size, 1234567890);
    assert.match(ticket.downloadUrl, new RegExp(`^http://127\\.0\\.0\\.1:${address.port}/d/.+`));
    assert.doesNotMatch(ticket.downloadUrl, /^https:/i);
    assert.equal(typeof ticket.expiresAt, "number");
    const skew = Math.abs(ticket.expiresAt - (Math.floor(Date.now() / 1000) + 900));
    assert.ok(skew < 5);
    assert.doesNotMatch(JSON.stringify(ticket), /password|\/Users\/|\/var\//i);

    assert.equal(
      (await request(server, {
        method: "POST",
        path: "/api/download-ticket",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ fileId: "missing" }),
      })).status,
      404,
    );
    assert.equal(
      (await request(server, {
        method: "POST",
        path: "/api/download-ticket",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ fileId: "d_sets" }),
      })).status,
      400,
    );
  } finally {
    server.close();
  }
});

test("streams a file and honors a single Range", async () => {
  const server = await start();
  try {
    const ticket = JSON.parse(
      (await request(server, {
        method: "POST",
        path: "/api/download-ticket",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ fileId: "f_readme" }),
      })).body.toString(),
    );
    const path = new URL(ticket.downloadUrl).pathname;
    const full = await request(server, { path });
    assert.equal(full.status, 200);
    assert.equal(full.headers["accept-ranges"], "bytes");
    assert.equal(Number(full.headers["content-length"]), 128);
    assert.equal(full.body.length, 128);
    assert.equal(full.body.every((byte) => byte === full.body[0]), false);

    const partial = await request(server, { path, headers: { range: "bytes=10-19" } });
    assert.equal(partial.status, 206);
    assert.equal(partial.headers["content-range"], "bytes 10-19/128");
    assert.equal(partial.body.length, 10);
    assert.deepEqual(partial.body, full.body.subarray(10, 20));

    const suffix = await request(server, { path, headers: { range: "bytes=-8" } });
    assert.equal(suffix.status, 206);
    assert.deepEqual(suffix.body, full.body.subarray(120));

    const open = await request(server, { path, headers: { range: "bytes=100-" } });
    assert.equal(open.status, 206);
    assert.deepEqual(open.body, full.body.subarray(100));

    const missing = await request(server, { path, headers: { range: "bytes=1000-1001" } });
    assert.equal(missing.status, 416);
    assert.equal(missing.headers["content-range"], "bytes */128");
  } finally {
    server.close();
  }
});

test("serves a high offset of the large file without the whole body", async () => {
  const server = await start();
  try {
    const ticket = JSON.parse(
      (await request(server, {
        method: "POST",
        path: "/api/download-ticket",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ fileId: "f_data" }),
      })).body.toString(),
    );
    const path = new URL(ticket.downloadUrl).pathname;
    const partial = await request(server, {
      path,
      headers: { range: "bytes=1000000-1000015" },
    });
    assert.equal(partial.status, 206);
    assert.equal(partial.headers["content-range"], "bytes 1000000-1000015/1234567890");
    assert.equal(Number(partial.headers["content-length"]), 16);
    assert.equal(partial.body.length, 16);
  } finally {
    server.close();
  }
});

test("rejects an expired token and a bad signature", async () => {
  const server = await start();
  try {
    const expired = signToken("f_readme", 1, SECRET);
    const expiredResponse = await request(server, { path: `/d/${expired}` });
    assert.equal(expiredResponse.status, 410);

    const valid = signToken("f_readme", Math.floor(Date.now() / 1000) + 60, SECRET);
    const [payload] = valid.split(".");
    const tampered = await request(server, { path: `/d/${payload}.not-a-signature` });
    assert.equal(tampered.status, 403);
  } finally {
    server.close();
  }
});

test("can ignore Range and can answer with X-Accel-Redirect", async () => {
  const plain = await start({ honorRange: false });
  try {
    const ticket = JSON.parse(
      (await request(plain, {
        method: "POST",
        path: "/api/download-ticket",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ fileId: "f_readme" }),
      })).body.toString(),
    );
    const path = new URL(ticket.downloadUrl).pathname;
    const response = await request(plain, { path, headers: { range: "bytes=0-3" } });
    assert.equal(response.status, 200);
    assert.equal(response.body.length, 128);
  } finally {
    plain.close();
  }

  const offload = await start({ xAccel: true });
  try {
    const ticket = JSON.parse(
      (await request(offload, {
        method: "POST",
        path: "/api/download-ticket",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ fileId: "f_readme" }),
      })).body.toString(),
    );
    const path = new URL(ticket.downloadUrl).pathname;
    const response = await request(offload, { path });
    assert.equal(response.status, 200);
    assert.equal(response.headers["x-accel-redirect"], "/internal/f_readme");
    assert.doesNotMatch(String(response.headers["x-accel-redirect"]), /\/Users\/|\/var\//);
    assert.equal(response.body.length, 0);
  } finally {
    offload.close();
  }
});

test("clamps ticket lifetime to 5–30 minutes", async () => {
  const server = await start({ ttlSeconds: 10 });
  try {
    const response = await request(server, {
      method: "POST",
      path: "/api/download-ticket",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ fileId: "f_sample" }),
    });
    const ticket = JSON.parse(response.body.toString());
    const skew = Math.abs(ticket.expiresAt - (Math.floor(Date.now() / 1000) + 300));
    assert.ok(skew < 5);
  } finally {
    server.close();
  }
});
