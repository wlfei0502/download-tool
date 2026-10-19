import assert from "node:assert/strict";
import { afterEach, describe, it } from "node:test";
import { download } from "../src/index.ts";

const API_BASE = "http://site.example";
const FILE_ID = "f_01hxyz";
const FILE_NAME = "data.zip";
const DOWNLOAD_URL = "http://127.0.0.1:9090/d/signed-token";
const PAGE = "http://site.example/files";
const REFERRER = "http://site.example/library";
const USER_AGENT = "Mozilla/5.0 Test";
const DEFAULT_THRESHOLD = 104857600;
const DEFAULT_PORT = 29128;

type FetchCall = {
  url: string;
  init?: RequestInit;
};

type AnchorClick = {
  href: string;
  download: string;
};

const originalFetch = globalThis.fetch;
const originalWindow = globalThis.window;
const originalDocument = globalThis.document;
const navigatorDescriptor = Object.getOwnPropertyDescriptor(globalThis, "navigator");

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function ticketBody(size = DEFAULT_THRESHOLD) {
  return {
    fileId: FILE_ID,
    fileName: FILE_NAME,
    size,
    downloadUrl: DOWNLOAD_URL,
    expiresAt: 1730000000,
  };
}

function header(init: RequestInit | undefined, name: string): string | null {
  const headers = init?.headers;
  if (!headers) return null;
  if (headers instanceof Headers) return headers.get(name);
  if (Array.isArray(headers)) {
    const found = headers.find(([key]) => key.toLowerCase() === name.toLowerCase());
    return found ? String(found[1]) : null;
  }
  const record = headers as Record<string, string>;
  const match = Object.entries(record).find(([key]) => key.toLowerCase() === name.toLowerCase());
  return match ? String(match[1]) : null;
}

function installFetch(
  handler: (url: string, init?: RequestInit) => Response | Promise<Response>,
): FetchCall[] {
  const calls: FetchCall[] = [];
  globalThis.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
    calls.push({ url, init });
    return handler(url, init);
  };
  return calls;
}

function installBrowser() {
  const hrefs: string[] = [];
  let current = PAGE;
  const location = {
    get href() {
      return current;
    },
    set href(value: string) {
      current = value;
      hrefs.push(value);
    },
  };
  const clicks: AnchorClick[] = [];
  const document = {
    referrer: REFERRER,
    body: {
      appendChild() {},
      removeChild() {},
    },
    createElement(tag: string) {
      assert.equal(tag, "a");
      const anchor = {
        href: "",
        download: "",
        rel: "",
        style: { display: "" },
        click() {
          clicks.push({ href: anchor.href, download: anchor.download });
        },
        remove() {},
      };
      return anchor;
    },
  };
  globalThis.window = { location, document } as unknown as Window & typeof globalThis;
  globalThis.document = document as unknown as Document;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    enumerable: true,
    writable: true,
    value: { userAgent: USER_AGENT },
  });
  return { location, clicks, hrefs };
}

afterEach(() => {
  globalThis.fetch = originalFetch;
  globalThis.window = originalWindow;
  globalThis.document = originalDocument;
  if (navigatorDescriptor) {
    Object.defineProperty(globalThis, "navigator", navigatorDescriptor);
  }
});

describe("download", () => {
  it("downloads small files via blob so cross-origin filenames are honored", async () => {
    const objectUrls: string[] = [];
    const originalCreate = URL.createObjectURL;
    const originalRevoke = URL.revokeObjectURL;
    URL.createObjectURL = ((blob: Blob) => {
      assert.ok(blob instanceof Blob);
      const url = `blob:http://site.example/${objectUrls.length}`;
      objectUrls.push(url);
      return url;
    }) as typeof URL.createObjectURL;
    URL.revokeObjectURL = ((url: string) => {
      assert.equal(url, objectUrls[0]);
    }) as typeof URL.revokeObjectURL;

    try {
      const calls = installFetch(async (url, init) => {
        if (url === `${API_BASE}/api/download-ticket`) {
          assert.equal(init?.method, "POST");
          assert.equal(header(init, "content-type"), "application/json");
          assert.deepEqual(JSON.parse(String(init?.body)), { fileId: FILE_ID });
          return jsonResponse(ticketBody(1024));
        }
        if (url === DOWNLOAD_URL) {
          assert.equal(init?.method, "GET");
          assert.equal(init?.mode, "cors");
          assert.equal(init?.credentials, "omit");
          return new Response("hello", {
            status: 200,
            headers: { "content-type": "text/plain" },
          });
        }
        throw new Error(`unexpected fetch ${url}`);
      });
      const { location, clicks } = installBrowser();

      const result = await download({
        fileId: FILE_ID,
        size: 1024,
        apiBase: `${API_BASE}/`,
      });

      assert.deepEqual(result, { mode: "browser" });
      assert.deepEqual(clicks, [{ href: "blob:http://site.example/0", download: FILE_NAME }]);
      assert.equal(location.href, PAGE);
      assert.equal(objectUrls.length, 1);
      assert.equal(
        calls.some((call) => call.url.includes("127.0.0.1:29128")),
        false,
      );
    } finally {
      URL.createObjectURL = originalCreate;
      URL.revokeObjectURL = originalRevoke;
    }
  });

  it("falls back to an anchor click when the file URL is not CORS-readable", async () => {
    installFetch(async (url) => {
      if (url.endsWith("/api/download-ticket")) return jsonResponse(ticketBody(1024));
      if (url === DOWNLOAD_URL) throw new TypeError("Failed to fetch");
      throw new Error(`unexpected fetch ${url}`);
    });
    const { clicks } = installBrowser();

    const result = await download({
      fileId: FILE_ID,
      size: 1024,
      apiBase: API_BASE,
    });

    assert.deepEqual(result, { mode: "browser" });
    assert.deepEqual(clicks, [{ href: DOWNLOAD_URL, download: FILE_NAME }]);
  });

  it("treats a file at the default threshold as a local download", async () => {
    const calls = installFetch(async (url) => {
      if (url.endsWith("/api/download-ticket")) return jsonResponse(ticketBody(DEFAULT_THRESHOLD));
      if (url === `http://127.0.0.1:${DEFAULT_PORT}/health`) {
        return jsonResponse({ ok: true, version: "0.1.0" });
      }
      if (url === `http://127.0.0.1:${DEFAULT_PORT}/v1/tasks`) {
        return jsonResponse({ taskId: "task-threshold" });
      }
      throw new Error(`unexpected ${url}`);
    });
    installBrowser();

    const result = await download({
      fileId: FILE_ID,
      size: DEFAULT_THRESHOLD,
      apiBase: API_BASE,
    });

    assert.deepEqual(result, { mode: "local", taskId: "task-threshold" });
    assert.equal(calls[1]?.url, `http://127.0.0.1:${DEFAULT_PORT}/health`);
    assert.equal(calls[1]?.init?.method ?? "GET", "GET");
  });

  it("keeps the browser path when size is below a custom threshold", async () => {
    const objectUrls: string[] = [];
    const originalCreate = URL.createObjectURL;
    URL.createObjectURL = (() => {
      const url = `blob:http://site.example/${objectUrls.length}`;
      objectUrls.push(url);
      return url;
    }) as typeof URL.createObjectURL;
    try {
      installFetch(async (url) => {
        if (url.endsWith("/api/download-ticket")) return jsonResponse(ticketBody(50));
        if (url === DOWNLOAD_URL) {
          return new Response("x", { status: 200, headers: { "content-type": "text/plain" } });
        }
        throw new Error(`unexpected fetch ${url}`);
      });
      const { clicks } = installBrowser();

      const result = await download({
        fileId: FILE_ID,
        size: 50,
        apiBase: API_BASE,
        config: { sizeThreshold: 100 },
      });

      assert.deepEqual(result, { mode: "browser" });
      assert.equal(clicks[0]?.href, "blob:http://site.example/0");
      assert.equal(clicks[0]?.download, FILE_NAME);
    } finally {
      URL.createObjectURL = originalCreate;
    }
  });

  it("skips the ticket API when downloadUrl and fileName are provided", async () => {
    const calls = installFetch(async (url, init) => {
      if (url.endsWith("/api/download-ticket")) {
        throw new Error("ticket must not be requested");
      }
      if (url.endsWith("/health")) return jsonResponse({ ok: true, version: "0.1.0" });
      if (url.endsWith("/v1/tasks")) {
        assert.deepEqual(JSON.parse(String(init?.body)), {
          url: DOWNLOAD_URL,
          fileName: FILE_NAME,
          size: 200_000_000,
          referrer: REFERRER,
          userAgent: USER_AGENT,
        });
        return jsonResponse({ taskId: "task-direct" });
      }
      throw new Error(`unexpected ${url}`);
    });
    installBrowser();

    const result = await download({
      downloadUrl: DOWNLOAD_URL,
      fileName: FILE_NAME,
      size: 200_000_000,
    });

    assert.deepEqual(result, { mode: "local", taskId: "task-direct" });
    assert.equal(calls.some((call) => call.url.includes("/api/download-ticket")), false);
  });

  it("posts a camelCase task when the local API is healthy", async () => {
    const calls = installFetch(async (url, init) => {
      if (url.endsWith("/api/download-ticket")) return jsonResponse(ticketBody(200_000_000));
      if (url.endsWith("/health")) return jsonResponse({ ok: true, version: "0.1.0" });
      if (url.endsWith("/v1/tasks")) {
        assert.equal(init?.method, "POST");
        assert.equal(header(init, "content-type"), "application/json");
        return jsonResponse({ taskId: "task-1" });
      }
      throw new Error(`unexpected ${url}`);
    });
    const { location } = installBrowser();

    const result = await download({
      fileId: FILE_ID,
      size: 200_000_000,
      apiBase: API_BASE,
    });

    assert.deepEqual(result, { mode: "local", taskId: "task-1" });
    assert.equal(calls[1]?.url, `http://127.0.0.1:${DEFAULT_PORT}/health`);
    assert.ok(calls[1]?.init?.signal);
    assert.equal(calls[2]?.url, `http://127.0.0.1:${DEFAULT_PORT}/v1/tasks`);
    assert.deepEqual(JSON.parse(String(calls[2]?.init?.body)), {
      url: DOWNLOAD_URL,
      fileName: FILE_NAME,
      size: 200_000_000,
      referrer: REFERRER,
      userAgent: USER_AGENT,
    });
    assert.equal(location.href, PAGE);
  });

  it("uses a configured local port", async () => {
    const calls = installFetch(async (url) => {
      if (url.endsWith("/api/download-ticket")) return jsonResponse(ticketBody(200_000_000));
      if (url === "http://127.0.0.1:39200/health") return jsonResponse({ ok: true });
      if (url === "http://127.0.0.1:39200/v1/tasks") return jsonResponse({ taskId: "task-port" });
      throw new Error(`unexpected ${url}`);
    });
    installBrowser();

    const result = await download({
      fileId: FILE_ID,
      size: 200_000_000,
      apiBase: API_BASE,
      config: { localPort: 39200 },
    });

    assert.deepEqual(result, { mode: "local", taskId: "task-port" });
    assert.equal(calls[1]?.url, "http://127.0.0.1:39200/health");
  });

  it("returns install when the protocol launch leaves health down", async () => {
    const calls = installFetch(async (url) => {
      if (url.endsWith("/api/download-ticket")) return jsonResponse(ticketBody(200_000_000));
      if (url.endsWith("/health")) throw new TypeError("Failed to fetch");
      throw new Error(`unexpected ${url}`);
    });
    const { hrefs } = installBrowser();
    let installCalls = 0;
    const started = Date.now();

    const result = await download({
      fileId: FILE_ID,
      size: 200_000_000,
      apiBase: API_BASE,
      config: {
        installUrl: "http://site.example/install",
        protocolSettleMs: 2000,
      },
      onNeedInstall() {
        installCalls += 1;
      },
    });

    const elapsed = Date.now() - started;
    assert.deepEqual(result, { mode: "install" });
    assert.equal(installCalls, 1);
    assert.equal(hrefs.length, 1);
    const opened = new URL(hrefs[0] ?? "");
    assert.equal(opened.protocol, "dltool:");
    assert.equal(opened.hostname, "add");
    assert.equal(opened.searchParams.get("url"), DOWNLOAD_URL);
    assert.equal(opened.searchParams.get("fileName"), FILE_NAME);
    assert.equal(opened.searchParams.get("size"), "200000000");
    assert.equal(opened.searchParams.get("referrer"), REFERRER);
    assert.equal(opened.searchParams.get("userAgent"), USER_AGENT);
    assert.equal(
      calls.some((call) => call.url.endsWith("/v1/tasks")),
      false,
    );
    assert.ok(elapsed >= 1500, `expected a brief health poll, took ${elapsed}ms`);
    assert.ok(elapsed < 5000, `protocol settle was too slow: ${elapsed}ms`);
  });

  it("opens installUrl when the app stays down and no callback is set", async () => {
    installFetch(async (url) => {
      if (url.endsWith("/api/download-ticket")) return jsonResponse(ticketBody(200_000_000));
      if (url.endsWith("/health")) throw new TypeError("Failed to fetch");
      throw new Error(`unexpected ${url}`);
    });
    const { hrefs, location } = installBrowser();

    const result = await download({
      fileId: FILE_ID,
      size: 200_000_000,
      apiBase: API_BASE,
      config: {
        installUrl: "http://site.example/get-the-app",
        protocolSettleMs: 400,
      },
    });

    assert.deepEqual(result, { mode: "install" });
    assert.equal(new URL(hrefs[0] ?? "").protocol, "dltool:");
    assert.equal(location.href, "http://site.example/get-the-app");
  });

  it("keeps protocol mode when health comes up after the scheme open", async () => {
    let healthAttempts = 0;
    const calls = installFetch(async (url) => {
      if (url.endsWith("/api/download-ticket")) return jsonResponse(ticketBody(200_000_000));
      if (url.endsWith("/health")) {
        healthAttempts += 1;
        if (healthAttempts === 1) throw new TypeError("Failed to fetch");
        return jsonResponse({ ok: true, aria2: true });
      }
      throw new Error(`unexpected ${url}`);
    });
    const { location } = installBrowser();
    let installCalls = 0;

    const result = await download({
      fileId: FILE_ID,
      size: 200_000_000,
      apiBase: API_BASE,
      config: { protocol: "customdl" },
      onNeedInstall() {
        installCalls += 1;
      },
    });

    assert.deepEqual(result, { mode: "protocol" });
    assert.equal(installCalls, 0);
    assert.equal(new URL(location.href).protocol, "customdl:");
    assert.equal(
      calls.some((call) => call.url.endsWith("/v1/tasks")),
      false,
    );
  });

  it("opens the protocol when health responds with an error status", async () => {
    installFetch(async (url) => {
      if (url.endsWith("/api/download-ticket")) return jsonResponse(ticketBody(200_000_000));
      if (url.endsWith("/health")) return jsonResponse({ ok: false }, 503);
      throw new Error(`unexpected ${url}`);
    });
    const { location } = installBrowser();

    const result = await download({
      fileId: FILE_ID,
      size: 200_000_000,
      apiBase: API_BASE,
    });

    assert.deepEqual(result, { mode: "protocol" });
    assert.equal(new URL(location.href).protocol, "dltool:");
  });

  it("returns install when the health probe times out", async () => {
    installFetch((url, init) => {
      if (url.endsWith("/api/download-ticket")) return jsonResponse(ticketBody(200_000_000));
      if (url.endsWith("/health")) {
        return new Promise((_resolve, reject) => {
          const abort = () => reject(new DOMException("The operation was aborted", "AbortError"));
          if (init?.signal?.aborted) {
            abort();
            return;
          }
          init?.signal?.addEventListener("abort", abort, { once: true });
        });
      }
      throw new Error(`unexpected ${url}`);
    });
    const { location } = installBrowser();
    let installCalls = 0;
    const started = Date.now();

    const result = await download({
      fileId: FILE_ID,
      size: 200_000_000,
      apiBase: API_BASE,
      config: { installUrl: "http://site.example/install" },
      onNeedInstall() {
        installCalls += 1;
      },
    });

    const elapsed = Date.now() - started;
    assert.deepEqual(result, { mode: "install" });
    assert.equal(installCalls, 1);
    assert.equal(location.href, PAGE);
    assert.ok(elapsed >= 350, `expected about 400ms, took ${elapsed}ms`);
    assert.ok(elapsed < 1500, `health timeout was too slow: ${elapsed}ms`);
  });

  it("opens installUrl when the probe times out and no callback is set", async () => {
    installFetch((url, init) => {
      if (url.endsWith("/api/download-ticket")) return jsonResponse(ticketBody(200_000_000));
      if (url.endsWith("/health")) {
        return new Promise((_resolve, reject) => {
          const abort = () => reject(new DOMException("The operation was aborted", "AbortError"));
          if (init?.signal?.aborted) {
            abort();
            return;
          }
          init?.signal?.addEventListener("abort", abort, { once: true });
        });
      }
      throw new Error(`unexpected ${url}`);
    });
    const { location } = installBrowser();

    const result = await download({
      fileId: FILE_ID,
      size: 200_000_000,
      apiBase: API_BASE,
      config: { installUrl: "http://site.example/get-the-app" },
    });

    assert.deepEqual(result, { mode: "install" });
    assert.equal(location.href, "http://site.example/get-the-app");
  });

  it("does not hand off locally when aria2 is reported down", async () => {
    const calls = installFetch(async (url) => {
      if (url.endsWith("/api/download-ticket")) return jsonResponse(ticketBody(200_000_000));
      if (url.endsWith("/health")) return jsonResponse({ ok: true, aria2: false });
      throw new Error(`unexpected ${url}`);
    });
    const { location } = installBrowser();

    const result = await download({
      fileId: FILE_ID,
      size: 200_000_000,
      apiBase: API_BASE,
    });

    assert.deepEqual(result, { mode: "protocol" });
    assert.equal(new URL(location.href).protocol, "dltool:");
    assert.equal(
      calls.some((call) => call.url.endsWith("/v1/tasks")),
      false,
    );
  });

  it("rejects a ticket whose download URL is neither http nor https", async () => {
    installFetch(async () =>
      jsonResponse({
        ...ticketBody(1024),
        downloadUrl: "ftp://files.example/d/token",
      }),
    );
    installBrowser();

    await assert.rejects(
      download({ fileId: FILE_ID, size: 1024, apiBase: API_BASE }),
      /http:\/\/ or https:\/\//,
    );
  });

  it("accepts a ticket whose download URL is https", async () => {
    installFetch(async (url) => {
      if (url.endsWith("/api/download-ticket")) {
        return jsonResponse({
          ...ticketBody(1024),
          downloadUrl: "https://files.example/d/token",
        });
      }
      throw new Error(`unexpected fetch ${url}`);
    });
    const { clicks } = installBrowser();

    const result = await download({ fileId: FILE_ID, size: 1024, apiBase: API_BASE });
    assert.deepEqual(result, { mode: "browser" });
    assert.equal(clicks[0]?.href, "https://files.example/d/token");
  });

  it("rejects a ticket whose download URL contains user info", async () => {
    installFetch(async () =>
      jsonResponse({
        ...ticketBody(1024),
        downloadUrl: "http://user:secret@127.0.0.1:9090/d/token",
      }),
    );
    installBrowser();

    await assert.rejects(
      download({ fileId: FILE_ID, size: 1024, apiBase: API_BASE }),
      /user info/,
    );
  });

  it("rejects a failed ticket request", async () => {
    installFetch(async () => jsonResponse({ error: "nope" }, 404));
    installBrowser();

    await assert.rejects(
      download({ fileId: FILE_ID, size: 1024, apiBase: API_BASE }),
      /download-ticket/,
    );
  });
});
