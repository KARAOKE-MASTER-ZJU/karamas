import type { PlaybackQuality } from "@/types/casting";

export type RoomEntryMode = "create" | "join"; // 房间进入模式类型
export type CastMode = "dlna" | "bilibili" | "local"; // 投屏模式类型
export type Theme = "system" | "light" | "dark"; // 主题类型
export type NoticeLevel = "info" | "warning" | "error"; // 操作提示级别类型
export type LogLevel = "trace" | "debug" | NoticeLevel; // 日志级别类型

// 连接配置接口
export interface ConnectionConfig {
  serverUrl: string;
  roomId: string;
}

// 用户偏好设置接口
export interface Preferences extends ConnectionConfig {
  /** 遥控注册使用的安装标识，首次启动生成后保持不变。 */
  appId: string;
  /** 是否允许网页遥控，默认关闭；下次连接生效。 */
  remoteControlEnabled: boolean;
  /** 网页遥控列表里显示的名字，留空使用系统默认。 */
  deviceName: string;
  theme: Theme;
  logLevel: LogLevel;
  dlnaProxyEnabled: boolean;
  dlnaHlsDirectEnabled: boolean;
  proxyDeviceOnly: boolean;
  bilibiliBuvid: string;
  mediaQuality: PlaybackQuality;
  bilibiliQuality: PlaybackQuality;
}

// 日志接口
export interface LogEntry {
  id: number;
  time: number;
  level: LogLevel;
  target: string;
  message: string;
}
