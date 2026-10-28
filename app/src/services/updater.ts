import { Channel, invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";

export interface UpdateInfo {
  version: string;
  notes: string;
}

export interface DownloadProgress {
  downloaded: number;
  total: number | null;
}

export const updaterApi = {
  version: getVersion,
  cancelDownload: () => invoke<void>("cancel_update_download"),
  supported: () => invoke<boolean>("update_supported"),
  check: () => invoke<UpdateInfo | null>("check_update"),
  download: async (onProgress: (progress: DownloadProgress) => void) => {
    const channel = new Channel<DownloadProgress>();
    channel.onmessage = onProgress;
    try {
      await invoke<void>("download_update", { onProgress: channel });
    } finally {
      channel.onmessage = () => {};
    }
  },
  install: () => invoke<void>("install_update"),
};
