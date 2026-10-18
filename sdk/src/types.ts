export type DownloadConfig = {
  /** Bytes. Files smaller than this use the browser. Default 104857600 (100 MiB). */
  sizeThreshold?: number;
  /** Opened when the local app does not come up and `onNeedInstall` is omitted. */
  installUrl?: string;
  /** Loopback port of the desktop app. Default 29128. */
  localPort?: number;
  /** Custom protocol scheme. Default "dltool". */
  protocol?: string;
};

export type DownloadOptions = {
  fileId: string;
  size: number;
  /** Website backend origin, without a trailing path. */
  apiBase: string;
  config?: DownloadConfig;
  /** Called when the health probe times out, or the app stays down after `dltool://`. */
  onNeedInstall?: () => void;
};

export type DownloadResult =
  | { mode: "browser" }
  | { mode: "local"; taskId: string }
  | { mode: "protocol" }
  | { mode: "install" };

/** Body returned by `POST /api/download-ticket`. `downloadUrl` is an http URL. */
export type DownloadTicket = {
  fileId: string;
  fileName: string;
  size: number;
  downloadUrl: string;
  expiresAt?: number;
};
