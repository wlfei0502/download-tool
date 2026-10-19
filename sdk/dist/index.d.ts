type DownloadConfig = {
    /** Bytes. Files smaller than this use the browser. Default 104857600 (100 MiB). */
    sizeThreshold?: number;
    /** Opened when the local app does not come up and `onNeedInstall` is omitted. */
    installUrl?: string;
    /** Loopback port of the desktop app. Default 29128. */
    localPort?: number;
    /** Custom protocol scheme. Default "dltool". */
    protocol?: string;
    /**
     * How long to poll `/health` after opening `dltool://` before install fallback.
     * Default 15000 — browsers often ask the user to confirm opening the app first.
     */
    protocolSettleMs?: number;
};
type DownloadOptions = {
    size: number;
    config?: DownloadConfig;
    /** Called when the health probe times out, or the app stays down after `dltool://`. */
    onNeedInstall?: () => void;
} & ({
    /** When set with `fileName`, skips `/api/download-ticket`. */
    downloadUrl: string;
    fileName: string;
    fileId?: string;
    apiBase?: string;
} | {
    /** Looked up via `POST {apiBase}/api/download-ticket`. */
    fileId: string;
    apiBase: string;
    downloadUrl?: undefined;
    fileName?: undefined;
});
type DownloadResult = {
    mode: "browser";
} | {
    mode: "local";
    taskId: string;
} | {
    mode: "protocol";
} | {
    mode: "install";
};
/** Body returned by `POST /api/download-ticket`. `downloadUrl` is an http URL. */
type DownloadTicket = {
    fileId: string;
    fileName: string;
    size: number;
    downloadUrl: string;
    expiresAt?: number;
};

declare const DEFAULT_SIZE_THRESHOLD = 104857600;
declare const DEFAULT_LOCAL_PORT = 29128;
declare const DEFAULT_PROTOCOL = "dltool";
/** How long to wait for the app after `dltool://` before showing install. */
declare const DEFAULT_PROTOCOL_SETTLE_MS = 15000;
/**
 * Hand a file to the browser, the local download app, or the install guide.
 *
 * Small files (`size < sizeThreshold`) download in the browser via Blob (so
 * cross-origin filenames still work when CORS allows). Larger files
 * probe `http://127.0.0.1:{port}/health`. A ready app (`ok` and aria2 not false)
 * receives `POST /v1/tasks`. A refused connection or a not-ready health body
 * opens `{protocol}://add?...` and polls `/health` briefly. If the app is still
 * unreachable, or the first probe times out, this calls `onNeedInstall` or
 * opens `installUrl`.
 */
declare function download(options: DownloadOptions): Promise<DownloadResult>;
declare const DownloadTool: {
    download: typeof download;
};

export { DEFAULT_LOCAL_PORT, DEFAULT_PROTOCOL, DEFAULT_PROTOCOL_SETTLE_MS, DEFAULT_SIZE_THRESHOLD, type DownloadConfig, type DownloadOptions, type DownloadResult, type DownloadTicket, DownloadTool, download };
