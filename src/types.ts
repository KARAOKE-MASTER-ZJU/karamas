export type RoomEntryMode = "create" | "join"; // 房间进入模式类型
export type CastMode = "dlna" | "bilibili"; // 投屏模式类型
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
  theme: Theme;
  logLevel: LogLevel;
  dlnaProxyEnabled: boolean;
}

// 日志接口
export interface LogEntry {
  id: number;
  time: number;
  level: LogLevel;
  target: string;
  message: string;
}
