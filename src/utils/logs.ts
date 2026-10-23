import type { LogEntry, LogLevel } from "@/types";

export const logLevelLabels = {
  trace: "trace",
  debug: "debug",
  info: "info",
  warning: "warning",
  error: "error",
} satisfies Record<LogLevel, string>;

export function isLogLevel(value: unknown): value is LogLevel {
  return (
    typeof value === "string" &&
    Object.prototype.hasOwnProperty.call(logLevelLabels, value)
  );
}

export function formatLogTime(time: number): string {
  return new Date(time).toLocaleTimeString("zh-CN", { hour12: false });
}

export function formatLogMessage(entry: LogEntry): string {
  return `[${entry.target}] ${entry.message}`;
}
