import type { DownloadOptions, DownloadResult, DownloadTicket } from "./types.ts";

export type { DownloadConfig, DownloadOptions, DownloadResult, DownloadTicket } from "./types.ts";

export const DEFAULT_SIZE_THRESHOLD = 104857600;
export const DEFAULT_LOCAL_PORT = 29128;
export const DEFAULT_PROTOCOL = "dltool";

const HEALTH_TIMEOUT_MS = 400;
/** How long to wait for the app after `dltool://` before showing install. */
const PROTOCOL_SETTLE_MS = 2000;
const PROTOCOL_POLL_GAP_MS = 200;

type Probe = "ok" | "down" | "unready" | "timeout";

type LocalTask = {
  url: string;
  fileName: string;
  size: number;
  referrer?: string;
  userAgent?: string;
};

type BrowserDocument = {
  referrer?: string;
  body?: { appendChild: (node: unknown) => void };
  createElement: (tag: string) => {
    href: string;
    download: string;
    rel: string;
    style?: { display: string };
    click: () => void;
    remove?: () => void;
  };
};

type BrowserWindow = {
  location: { href: string };
  document?: BrowserDocument;
};

/**
 * Hand a file to the browser, the local download app, or the install guide.
 *
 * Small files (`size < sizeThreshold`) download in the browser. Larger files
 * probe `http://127.0.0.1:{port}/health`. A ready app (`ok` and aria2 not false)
 * receives `POST /v1/tasks`. A refused connection or a not-ready health body
 * opens `{protocol}://add?...` and polls `/health` briefly. If the app is still
 * unreachable, or the first probe times out, this calls `onNeedInstall` or
 * opens `installUrl`.
 */
export async function download(options: DownloadOptions): Promise<DownloadResult> {
  const threshold = options.config?.sizeThreshold ?? DEFAULT_SIZE_THRESHOLD;
  const port = options.config?.localPort ?? DEFAULT_LOCAL_PORT;
  const protocol = options.config?.protocol ?? DEFAULT_PROTOCOL;
  const ticket = await resolveTicket(options);

  if (ticket.size < threshold) {
    triggerBrowserDownload(ticket.downloadUrl, ticket.fileName);
    return { mode: "browser" };
  }

  const probe = await probeHealth(port);
  if (probe === "ok") {
    const taskId = await createLocalTask(port, toLocalTask(ticket));
    return { mode: "local", taskId };
  }

  if (probe === "timeout") {
    return installResult(options);
  }

  navigate(buildProtocolUrl(protocol, toLocalTask(ticket)));
  const settled = await settleAfterProtocol(port);
  if (settled === "down") {
    return installResult(options);
  }
  return { mode: "protocol" };
}

function installResult(options: DownloadOptions): DownloadResult {
  options.onNeedInstall?.();
  const installUrl = options.config?.installUrl;
  if (!options.onNeedInstall && installUrl) {
    navigate(installUrl);
  }
  return { mode: "install" };
}

export const DownloadTool = { download };

async function resolveTicket(options: DownloadOptions): Promise<DownloadTicket> {
  if (options.downloadUrl && options.fileName) {
    return parseTicket(
      {
        fileId: options.fileId ?? "",
        fileName: options.fileName,
        size: options.size,
        downloadUrl: options.downloadUrl,
      },
      options.size,
    );
  }
  if (!options.apiBase || !options.fileId) {
    throw new Error("provide downloadUrl+fileName, or fileId+apiBase");
  }
  return requestTicket(options.apiBase, options.fileId, options.size);
}

async function requestTicket(apiBase: string, fileId: string, fallbackSize: number): Promise<DownloadTicket> {
  const url = `${apiBase.replace(/\/+$/, "")}/api/download-ticket`;
  const body = await postJson(url, { fileId }, "include");
  return parseTicket(body, fallbackSize);
}

function parseTicket(value: unknown, fallbackSize: number): DownloadTicket {
  if (!value || typeof value !== "object") {
    throw new Error("download ticket response was not an object");
  }
  const body = value as Record<string, unknown>;
  if (typeof body.downloadUrl !== "string" || body.downloadUrl.length === 0) {
    throw new Error("download ticket missing downloadUrl");
  }
  const downloadUrl = body.downloadUrl.toLowerCase();
  if (!downloadUrl.startsWith("http://") && !downloadUrl.startsWith("https://")) {
    throw new Error("download ticket URL must use http:// or https://");
  }
  if (httpUrlHasUserInfo(body.downloadUrl)) {
    throw new Error("download ticket URL must not include user info");
  }
  if (typeof body.fileName !== "string" || body.fileName.length === 0) {
    throw new Error("download ticket missing fileName");
  }
  const size =
    typeof body.size === "number" && Number.isFinite(body.size) ? body.size : fallbackSize;
  return {
    fileId: typeof body.fileId === "string" ? body.fileId : "",
    fileName: body.fileName,
    size,
    downloadUrl: body.downloadUrl,
    expiresAt: typeof body.expiresAt === "number" ? body.expiresAt : undefined,
  };
}

function toLocalTask(ticket: DownloadTicket): LocalTask {
  const task: LocalTask = {
    url: ticket.downloadUrl,
    fileName: ticket.fileName,
    size: ticket.size,
  };
  const referrer = pageReferrer();
  const userAgent = pageUserAgent();
  if (referrer) task.referrer = referrer;
  if (userAgent) task.userAgent = userAgent;
  return task;
}

async function probeHealth(port: number): Promise<Probe> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), HEALTH_TIMEOUT_MS);
  try {
    const response = await globalThis.fetch(`http://127.0.0.1:${port}/health`, {
      method: "GET",
      signal: controller.signal,
      cache: "no-store",
      credentials: "omit",
    });
    if (!response.ok) return "unready";
    const body = await response.json().catch(() => null);
    return healthReady(body) ? "ok" : "unready";
  } catch (error) {
    if (controller.signal.aborted || isAbortError(error)) return "timeout";
    return "down";
  } finally {
    clearTimeout(timer);
  }
}

function healthReady(body: unknown): boolean {
  if (!body || typeof body !== "object") return false;
  const record = body as { ok?: unknown; aria2?: unknown };
  return record.ok === true && record.aria2 !== false;
}

function httpUrlHasUserInfo(value: string): boolean {
  const match = /^https?:\/\//i.exec(value);
  if (!match) return false;
  const authority = value.slice(match[0].length).split(/[/?#]/, 1)[0] ?? "";
  return authority.includes("@");
}

async function settleAfterProtocol(port: number): Promise<"ok" | "unready" | "down"> {
  const deadline = Date.now() + PROTOCOL_SETTLE_MS;
  while (true) {
    const probe = await probeHealth(port);
    if (probe === "ok") return "ok";
    if (probe === "unready") return "unready";
    if (Date.now() >= deadline) return "down";
    const pause = Math.min(PROTOCOL_POLL_GAP_MS, Math.max(0, deadline - Date.now()));
    if (pause === 0) return "down";
    await delay(pause);
  }
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

async function createLocalTask(port: number, task: LocalTask): Promise<string> {
  const body = await postJson(`http://127.0.0.1:${port}/v1/tasks`, task, "omit");
  if (!body || typeof body !== "object" || typeof (body as { taskId?: unknown }).taskId !== "string") {
    throw new Error("local task response missing taskId");
  }
  const taskId = (body as { taskId: string }).taskId;
  if (taskId.length === 0) throw new Error("local task response missing taskId");
  return taskId;
}

function buildProtocolUrl(protocol: string, task: LocalTask): string {
  const params = new URLSearchParams();
  params.set("url", task.url);
  params.set("fileName", task.fileName);
  params.set("size", String(task.size));
  if (task.referrer) params.set("referrer", task.referrer);
  if (task.userAgent) params.set("userAgent", task.userAgent);
  return `${protocol}://add?${params.toString()}`;
}

async function postJson(url: string, body: unknown, credentials: RequestCredentials): Promise<unknown> {
  const response = await globalThis.fetch(url, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      accept: "application/json",
    },
    body: JSON.stringify(body),
    credentials,
  });
  if (!response.ok) {
    throw new Error(`POST ${url} failed with status ${response.status}`);
  }
  return response.json() as Promise<unknown>;
}

function triggerBrowserDownload(downloadUrl: string, fileName: string): void {
  const doc = readDocument();
  if (!doc) {
    navigate(downloadUrl);
    return;
  }
  const anchor = doc.createElement("a");
  anchor.href = downloadUrl;
  anchor.download = fileName;
  anchor.rel = "noopener";
  if (anchor.style) anchor.style.display = "none";
  doc.body?.appendChild(anchor);
  anchor.click();
  anchor.remove?.();
}

function navigate(href: string): void {
  const win = readWindow();
  if (!win) throw new Error("navigation requires window.location");
  win.location.href = href;
}

function readWindow(): BrowserWindow | undefined {
  const win = (globalThis as { window?: BrowserWindow }).window;
  if (win?.location) return win;
  return undefined;
}

function readDocument(): BrowserDocument | undefined {
  const fromGlobal = (globalThis as { document?: BrowserDocument }).document;
  if (fromGlobal?.createElement) return fromGlobal;
  return readWindow()?.document;
}

function pageReferrer(): string | undefined {
  const referrer = readDocument()?.referrer?.trim();
  if (referrer) return referrer;
  const href = readWindow()?.location.href?.trim();
  if (href && /^https?:\/\//i.test(href)) return href;
  return undefined;
}

function pageUserAgent(): string | undefined {
  const userAgent = (globalThis as { navigator?: { userAgent?: string } }).navigator?.userAgent?.trim();
  return userAgent ? userAgent : undefined;
}

function isAbortError(error: unknown): boolean {
  return error instanceof Error && error.name === "AbortError";
}
