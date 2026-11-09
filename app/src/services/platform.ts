import {
  Channel as TauriChannel,
  invoke as tauriInvoke,
  isTauri as tauriIsTauri,
} from "@tauri-apps/api/core";
import {
  emit as tauriEmit,
  listen as tauriListen,
  type UnlistenFn,
} from "@tauri-apps/api/event";
import {
  getVersion as tauriVersion,
  onBackButtonPress as tauriBack,
  supportsMultipleWindows as tauriMultipleWindows,
} from "@tauri-apps/api/app";
import { openUrl as tauriOpenUrl } from "@tauri-apps/plugin-opener";

declare global {
  interface Window {
    __KARAMAS_PRESENTATION_SUPPORTED__?: boolean;
    KaramasController?: { postMessage: (message: string) => void };
  }
}

type ControllerCall = { token: string; id: string } & (
  | { type: "invoke"; command: string; args?: Record<string, unknown> }
  | { type: "listen"; event: string; callback: string }
  | { type: "unlisten"; subscription: string }
  | { type: "emit"; event: string; payload: unknown }
  | { type: "back"; callback: string }
  | { type: "version" }
  | { type: "openUrl"; url: string }
);
type Request = ControllerCall extends infer T
  ? T extends ControllerCall
    ? Omit<T, "id" | "token">
    : never
  : never;
type Reply =
  | { id: string; value?: unknown; error?: unknown }
  | { callback: string; value: unknown };
const pending = new Map<
  string,
  { resolve: (value: unknown) => void; reject: (error: unknown) => void }
>();
const callbacks = new Map<string, (value: unknown) => void>();
const subscriptions = new Map<string, () => void | Promise<void>>();
const channels = new Map<string, TauriChannel<unknown>>();

function request<T>(message: Request): Promise<T> {
  return new Promise((resolve, reject) => {
    const id = crypto.randomUUID();
    pending.set(id, { resolve: (value) => resolve(value as T), reject });
    try {
      window.KaramasController!.postMessage(JSON.stringify({ ...message, id }));
    } catch (error) {
      pending.delete(id);
      reject(error);
    }
  });
}

window.addEventListener("android-controller-message", (event) => {
  const reply = (event as CustomEvent<Reply>).detail;
  if ("callback" in reply) {
    callbacks.get(reply.callback)?.(reply.value);
    return;
  }
  const task = pending.get(reply.id);
  if (!task) return;
  pending.delete(reply.id);
  if (reply.error) task.reject(reply.error);
  else task.resolve(reply.value);
});

function send(token: string, message: Reply) {
  return tauriInvoke("send_android_controller_message", { token, message });
}

// Android 控制文档没有 Tauri 注入，通过仍存活的 RustWebView 接入原有原生命令。
// Channel 和监听器在原播放文档中注册，控制文档退出时统一释放。
function nativeArgs(value: unknown, token: string): unknown {
  if (value && typeof value === "object" && "androidChannel" in value) {
    const id = String(value.androidChannel);
    let channel = channels.get(id);
    if (!channel) {
      channel = new TauriChannel((payload) => {
        void send(token, { callback: id, value: payload });
      });
      channels.set(id, channel);
    }
    return channel;
  }
  if (Array.isArray(value)) return value.map((item) => nativeArgs(item, token));
  if (value && typeof value === "object")
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [
        key,
        nativeArgs(item, token),
      ]),
    );
  return value;
}

window.addEventListener("android-controller-call", (event) => {
  const call = (event as CustomEvent<ControllerCall>).detail;
  const work = async () => {
    switch (call.type) {
      case "invoke":
        return tauriInvoke(
          call.command,
          nativeArgs(call.args, call.token) as Record<string, unknown>,
        );
      case "version":
        return tauriVersion();
      case "openUrl":
        return tauriOpenUrl(call.url);
      case "emit":
        return tauriEmit(call.event, call.payload);
      case "listen": {
        const dispose = await tauriListen(call.event, (payload) => {
          void send(call.token, { callback: call.callback, value: payload });
        });
        subscriptions.set(call.id, dispose);
        return call.id;
      }
      case "back": {
        const listener = await tauriBack((payload) => {
          void send(call.token, { callback: call.callback, value: payload });
        });
        subscriptions.set(call.id, () => listener.unregister());
        return call.id;
      }
      case "unlisten": {
        await subscriptions.get(call.subscription)?.();
        subscriptions.delete(call.subscription);
        return;
      }
    }
  };
  void work().then(
    (value) => send(call.token, { id: call.id, value }),
    (error: unknown) => send(call.token, { id: call.id, error }),
  );
});
window.addEventListener("android-controller-reset", () => {
  for (const dispose of subscriptions.values()) void dispose();
  subscriptions.clear();
  for (const channel of channels.values()) channel.onmessage = () => {};
  channels.clear();
});

export function isTauri() {
  return Boolean(window.KaramasController) || tauriIsTauri();
}
export function invoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  return window.KaramasController
    ? request({ type: "invoke", command, args })
    : tauriInvoke(command, args);
}

export class Channel<T> {
  private native: TauriChannel<T> | undefined;
  private callback = crypto.randomUUID();
  private handler: (value: T) => void = () => {};
  constructor() {
    if (window.KaramasController)
      callbacks.set(this.callback, (value) => this.handler(value as T));
    else this.native = new TauriChannel((value) => this.handler(value));
  }
  set onmessage(handler: (value: T) => void) {
    this.handler = handler;
  }
  toJSON() {
    return this.native?.toJSON() ?? { androidChannel: this.callback };
  }
  dispose() {
    this.handler = () => {};
    callbacks.delete(this.callback);
  }
}

export async function listen<T>(
  event: string,
  callback: (event: { payload: T }) => void,
): Promise<UnlistenFn> {
  if (!window.KaramasController) return tauriListen<T>(event, callback);
  const id = crypto.randomUUID();
  callbacks.set(id, (value) => callback(value as { payload: T }));
  const subscription = await request<string>({
    type: "listen",
    event,
    callback: id,
  });
  return () => {
    callbacks.delete(id);
    void request({ type: "unlisten", subscription });
  };
}
export function emit(event: string, payload?: unknown): Promise<void> {
  return window.KaramasController
    ? request({ type: "emit", event, payload })
    : tauriEmit(event, payload);
}
export async function onBackButtonPress(callback: () => void) {
  if (!window.KaramasController) return tauriBack(callback);
  const id = crypto.randomUUID();
  callbacks.set(id, callback);
  const subscription = await request<string>({ type: "back", callback: id });
  return {
    unregister: async () => {
      callbacks.delete(id);
      await request({ type: "unlisten", subscription });
    },
  };
}
export function getVersion(): Promise<string> {
  return window.KaramasController
    ? request({ type: "version" })
    : tauriVersion();
}
export function openUrl(url: string): Promise<void> {
  return window.KaramasController
    ? request({ type: "openUrl", url })
    : tauriOpenUrl(url);
}
export function supportsMultipleWindows(): Promise<boolean> {
  return window.KaramasController
    ? Promise.resolve(false)
    : tauriMultipleWindows();
}

window.addEventListener("android-playback-returned", () => {
  void tauriInvoke("android_playback_returned");
});
