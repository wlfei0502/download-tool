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
  const location = { href: PAGE };
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
  return { location, clicks };
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
  it("downloads small files in the browser from the ticket URL", async () => {
    const calls = installFetch(async (url, init) => {
      assert.equal(url, `${API_BASE}/api/download-ticket`);
      assert.equal(init?.method, "POST");
      assert.equal(header(init, "content-type"), "application/json");
      assert.deepEqual(JSON.parse(String(init?.body)), { fileId: FILE_ID });
      return jsonResponse(ticketBody(1024));
    });
    const { location, clicks } = installBrowser();

    const result = await download({
      fileId: FILE_ID,
      size: 1024,
      apiBase: `${API_BASE}/`,
    });

    assert.deepEqual(result, { mode: "browser" });
    assert.deepEqual(clicks, [{ href: DOWNLOAD_URL, download: FILE_NAME }]);
    assert.equal(location.href, PAGE);
    assert.equal(calls.length, 1);
    assert.equal(
      calls.some((call) => call.url.includes("127.0.0.1")),
      false,
    );
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
    installFetch(async () => jsonResponse(ticketBody(50)));
    const { clicks } = installBrowser();

    const result = await download({
      fileId: FILE_ID,
      size: 50,
      apiBase: API_BASE,
      config: { sizeThreshold: 100 },
    });

    assert.deepEqual(result, { mode: "browser" });
    assert.equal(clicks[0]?.href, DOWNLOAD_URL);
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

  it("invokes the dltool protocol when health is unreachable", async () => {
    const calls = installFetch(async (url) => {
      if (url.endsWith("/api/download-ticket")) return jsonResponse(ticketBody(200_000_000));
      if (url.endsWith("/health")) throw new TypeError("Failed to fetch");
      throw new Error(`unexpected ${url}`);
    });
    const { location } = installBrowser();
    let installCalls = 0;

    const result = await download({
      fileId: FILE_ID,
      size: 200_000_000,
      apiBase: API_BASE,
      onNeedInstall() {
        installCalls += 1;
      },
    });

    assert.deepEqual(result, { mode: "protocol" });
    assert.equal(installCalls, 0);
    const opened = new URL(location.href);
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
  });

  it("uses a configured protocol scheme", async () => {
    installFetch(async (url) => {
      if (url.endsWith("/api/download-ticket")) return jsonResponse(ticketBody(200_000_000));
      throw new TypeError("Failed to fetch");
    });
    const { location } = installBrowser();

    const result = await download({
      fileId: FILE_ID,
      size: 200_000_000,
      apiBase: API_BASE,
      config: { protocol: "customdl" },
    });

    assert.deepEqual(result, { mode: "protocol" });
    assert.equal(new URL(location.href).protocol, "customdl:");
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

  it("rejects a ticket whose download URL is not http", async () => {
    installFetch(async () =>
      jsonResponse({
        ...ticketBody(1024),
        downloadUrl: "https://files.example/d/token",
      }),
    );
    installBrowser();

    await assert.rejects(
      download({ fileId: FILE_ID, size: 1024, apiBase: API_BASE }),
      /http:\/\//,
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
