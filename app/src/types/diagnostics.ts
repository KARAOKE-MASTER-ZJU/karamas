import type { MediaFormat } from "@/types/casting";

export const diagnosticSections = [
  { key: "device", title: "设备信息" },
  { key: "functions", title: "设备功能测试" },
  { key: "parameters", title: "参数查询" },
  { key: "playback", title: "播放测试" },
  { key: "latency", title: "延迟测试" },
] as const;
export type DiagnosticKind = (typeof diagnosticSections)[number]["key"];
export interface TestResponse {
  time: number;
  url: string;
  request: string;
  responseHeaders: string;
  responseUrl: string | null;
  status: number | null;
  elapsedMs: number;
  fields: Record<string, string>;
  raw: string;
  error: string | null;
}
export interface DiagnosticService {
  serviceType: string;
  serviceId: string;
  scpdUrl: string;
  controlUrl: string;
  eventSubUrl: string;
}
export interface DeviceInfo {
  response: TestResponse;
  services: DiagnosticService[];
}
export interface DiagnosticEntry {
  title: string;
  result: TestResponse;
  services?: DiagnosticService[];
}
export interface DiagnosticPanelProps {
  location: string;
  run: (work: (current: () => boolean) => Promise<void>) => Promise<void>;
}
export interface FunctionCheck {
  title: string;
  service: string;
  action: string;
  state: "running" | "passed" | "failed" | "undeclared" | "skipped";
  detail: string;
  response: TestResponse | null;
}
export interface DiagnosticArgument {
  name: string;
  direction: string;
  relatedStateVariable: string;
}
export interface DiagnosticVariable {
  name: string;
  dataType: string;
  defaultValue: string;
  allowedValues: string[];
  minimum: string;
  maximum: string;
  step: string;
}
export interface ServiceInfo {
  response: TestResponse;
  actions: { name: string; arguments: DiagnosticArgument[] }[];
  stateVariables: DiagnosticVariable[];
}
export type PlaybackTest =
  | { action: "load"; songUrl: string; format: MediaFormat; direct: boolean }
  | { action: "play" | "pause" | "stop" | "read" | "readVolume" }
  | { action: "seek"; seconds: number }
  | { action: "volume"; volume: number };
