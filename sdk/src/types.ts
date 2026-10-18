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
  size: number;
  config?: DownloadConfig;
  /** Called when the health probe times out, or the app stays down after `dltool://`. */
  onNeedInstall?: () => void;
} & (
  | {
      /** When set with `fileName`, skips `/api/download-ticket`. */
      downloadUrl: string;
      fileName: string;
      fileId?: string;
      apiBase?: string;
    }
  | {
      /** Looked up via `POST {apiBase}/api/download-ticket`. */
      fileId: string;
      apiBase: string;
      downloadUrl?: undefined;
      fileName?: undefined;
    }
);

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
