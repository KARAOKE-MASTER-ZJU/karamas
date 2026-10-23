import { openUrl } from "@tauri-apps/plugin-opener";
import { isTauri } from "@tauri-apps/api/core";
import type { ConnectionConfig } from "@/types";

export async function openSongRoom(room: ConnectionConfig) {
  const url = new URL(`${room.serverUrl.replace(/\/$/, "")}/room`);
  if (
    !["http:", "https:"].includes(url.protocol) ||
    url.username ||
    url.password
  )
    throw new Error("请输入有效的 HTTP 或 HTTPS 服务器网址。");
  url.searchParams.set("roomId", room.roomId);
  if (isTauri()) await openUrl(url.toString());
  else window.open(url.toString(), "_blank", "noopener,noreferrer");
}
