import type { LogLevel } from "@/types";

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

const severity: Record<LogLevel, number> = {
  trace: 0,
  debug: 1,
  info: 2,
  warning: 3,
  error: 4,
};

export function meetsLogLevel(level: LogLevel, minimum: LogLevel): boolean {
  return severity[level] >= severity[minimum];
}
