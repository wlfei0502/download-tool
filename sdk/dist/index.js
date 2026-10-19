// src/index.ts
var DEFAULT_SIZE_THRESHOLD = 104857600;
var DEFAULT_LOCAL_PORT = 29128;
var DEFAULT_PROTOCOL = "dltool";
var HEALTH_TIMEOUT_MS = 400;
var DEFAULT_PROTOCOL_SETTLE_MS = 15e3;
var PROTOCOL_POLL_GAP_MS = 200;
async function download(options) {
  const threshold = options.config?.sizeThreshold ?? DEFAULT_SIZE_THRESHOLD;
  const port = options.config?.localPort ?? DEFAULT_LOCAL_PORT;
  const protocol = options.config?.protocol ?? DEFAULT_PROTOCOL;
  const ticket = await resolveTicket(options);
  if (ticket.size < threshold) {
    await triggerBrowserDownload(ticket.downloadUrl, ticket.fileName);
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
  const settleMs = options.config?.protocolSettleMs ?? DEFAULT_PROTOCOL_SETTLE_MS;
  const settled = await settleAfterProtocol(port, settleMs);
  if (settled === "down") {
    return installResult(options);
  }
  return { mode: "protocol" };
}
function installResult(options) {
  options.onNeedInstall?.();
  const installUrl = options.config?.installUrl;
  if (!options.onNeedInstall && installUrl) {
    navigate(installUrl);
  }
  return { mode: "install" };
}
var DownloadTool = { download };
async function resolveTicket(options) {
  if (options.downloadUrl && options.fileName) {
    return parseTicket(
      {
        fileId: options.fileId ?? "",
        fileName: options.fileName,
        size: options.size,
        downloadUrl: options.downloadUrl
      },
      options.size
    );
  }
  if (!options.apiBase || !options.fileId) {
    throw new Error("provide downloadUrl+fileName, or fileId+apiBase");
  }
  return requestTicket(options.apiBase, options.fileId, options.size);
}
async function requestTicket(apiBase, fileId, fallbackSize) {
  const url = `${apiBase.replace(/\/+$/, "")}/api/download-ticket`;
  const body = await postJson(url, { fileId }, "include");
  return parseTicket(body, fallbackSize);
}
function parseTicket(value, fallbackSize) {
  if (!value || typeof value !== "object") {
    throw new Error("download ticket response was not an object");
  }
  const body = value;
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
  const size = typeof body.size === "number" && Number.isFinite(body.size) ? body.size : fallbackSize;
  return {
    fileId: typeof body.fileId === "string" ? body.fileId : "",
    fileName: body.fileName,
    size,
    downloadUrl: body.downloadUrl,
    expiresAt: typeof body.expiresAt === "number" ? body.expiresAt : void 0
  };
}
function toLocalTask(ticket) {
  const task = {
    url: ticket.downloadUrl,
    fileName: ticket.fileName,
    size: ticket.size
  };
  const referrer = pageReferrer();
  const userAgent = pageUserAgent();
  if (referrer) task.referrer = referrer;
  if (userAgent) task.userAgent = userAgent;
  return task;
}
async function probeHealth(port) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), HEALTH_TIMEOUT_MS);
  try {
    const response = await globalThis.fetch(`http://127.0.0.1:${port}/health`, {
      method: "GET",
      signal: controller.signal,
      cache: "no-store",
      credentials: "omit"
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
function healthReady(body) {
  if (!body || typeof body !== "object") return false;
  const record = body;
  return record.ok === true && record.aria2 !== false;
}
function httpUrlHasUserInfo(value) {
  const match = /^https?:\/\//i.exec(value);
  if (!match) return false;
  const authority = value.slice(match[0].length).split(/[/?#]/, 1)[0] ?? "";
  return authority.includes("@");
}
async function settleAfterProtocol(port, settleMs) {
  const budget = Math.max(0, settleMs);
  const deadline = Date.now() + budget;
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
function delay(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}
async function createLocalTask(port, task) {
  const body = await postJson(`http://127.0.0.1:${port}/v1/tasks`, task, "omit");
  if (!body || typeof body !== "object" || typeof body.taskId !== "string") {
    throw new Error("local task response missing taskId");
  }
  const taskId = body.taskId;
  if (taskId.length === 0) throw new Error("local task response missing taskId");
  return taskId;
}
function buildProtocolUrl(protocol, task) {
  const params = new URLSearchParams();
  params.set("url", task.url);
  params.set("fileName", task.fileName);
  params.set("size", String(task.size));
  if (task.referrer) params.set("referrer", task.referrer);
  if (task.userAgent) params.set("userAgent", task.userAgent);
  return `${protocol}://add?${params.toString()}`;
}
async function postJson(url, body, credentials) {
  const response = await globalThis.fetch(url, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      accept: "application/json"
    },
    body: JSON.stringify(body),
    credentials
  });
  if (!response.ok) {
    throw new Error(`POST ${url} failed with status ${response.status}`);
  }
  return response.json();
}
async function triggerBrowserDownload(downloadUrl, fileName) {
  const doc = readDocument();
  if (!doc) {
    navigate(downloadUrl);
    return;
  }
  try {
    const response = await globalThis.fetch(downloadUrl, {
      method: "GET",
      mode: "cors",
      credentials: sameOrigin(downloadUrl) ? "include" : "omit",
      cache: "no-store"
    });
    if (!response.ok) {
      throw new Error(`GET ${downloadUrl} failed with status ${response.status}`);
    }
    const blob = await response.blob();
    const objectUrl = URL.createObjectURL(blob);
    try {
      clickDownloadAnchor(doc, objectUrl, fileName);
    } finally {
      URL.revokeObjectURL(objectUrl);
    }
  } catch {
    clickDownloadAnchor(doc, downloadUrl, fileName);
  }
}
function clickDownloadAnchor(doc, href, fileName) {
  const anchor = doc.createElement("a");
  anchor.href = href;
  anchor.download = fileName;
  anchor.rel = "noopener";
  if (anchor.style) anchor.style.display = "none";
  doc.body?.appendChild(anchor);
  anchor.click();
  anchor.remove?.();
}
function sameOrigin(url) {
  try {
    const page = readWindow()?.location.href;
    if (!page) return false;
    return new URL(url, page).origin === new URL(page).origin;
  } catch {
    return false;
  }
}
function navigate(href) {
  const win = readWindow();
  if (!win) throw new Error("navigation requires window.location");
  win.location.href = href;
}
function readWindow() {
  const win = globalThis.window;
  if (win?.location) return win;
  return void 0;
}
function readDocument() {
  const fromGlobal = globalThis.document;
  if (fromGlobal?.createElement) return fromGlobal;
  return readWindow()?.document;
}
function pageReferrer() {
  const referrer = readDocument()?.referrer?.trim();
  if (referrer) return referrer;
  const href = readWindow()?.location.href?.trim();
  if (href && /^https?:\/\//i.test(href)) return href;
  return void 0;
}
function pageUserAgent() {
  const userAgent = globalThis.navigator?.userAgent?.trim();
  return userAgent ? userAgent : void 0;
}
function isAbortError(error) {
  return error instanceof Error && error.name === "AbortError";
}
export {
  DEFAULT_LOCAL_PORT,
  DEFAULT_PROTOCOL,
  DEFAULT_PROTOCOL_SETTLE_MS,
  DEFAULT_SIZE_THRESHOLD,
  DownloadTool,
  download
};
//# sourceMappingURL=index.js.map