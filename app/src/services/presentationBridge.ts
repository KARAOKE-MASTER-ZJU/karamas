import type { UnlistenFn } from "@tauri-apps/api/event";

declare global {
  interface Window {
    __KARAMAS_PRESENTATION_SUPPORTED__?: boolean;
    KaramasPlayback?: {
      postMessage: (message: string) => void;
    };
  }
}

const requests = new Map<
  string,
  { resolve: (value: unknown) => void; reject: (error: unknown) => void }
>();

function listenPresentationMessage<T>(
  type: "command" | "reply",
  callback: (payload: T) => void,
): UnlistenFn {
  const listener = (event: Event) => {
    const message = (event as CustomEvent<{ type: string; payload: T }>).detail;
    if (message.type === type) callback(message.payload);
  };
  window.addEventListener("playback-window-message", listener);
  return () => window.removeEventListener("playback-window-message", listener);
}

if (window.KaramasPlayback) {
  listenPresentationMessage<{ id: string; value?: unknown; error?: unknown }>(
    "reply",
    (reply) => {
      const request = requests.get(reply.id);
      if (!request) return;
      requests.delete(reply.id);
      if (reply.error) request.reject(reply.error);
      else request.resolve(reply.value);
    },
  );
}

export function invokePresentation<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  return new Promise((resolve, reject) => {
    const id = crypto.randomUUID();
    requests.set(id, { resolve: (value) => resolve(value as T), reject });
    try {
      window.KaramasPlayback!.postMessage(
        JSON.stringify({ type: "invoke", id, command, args }),
      );
    } catch (error) {
      requests.delete(id);
      reject(error);
    }
  });
}

export async function sendPresentationMessage(message: unknown): Promise<void> {
  window.KaramasPlayback!.postMessage(
    JSON.stringify({ type: "message", message }),
  );
}

export function listenPresentationCommand<T>(
  callback: (event: { payload: T }) => void,
): UnlistenFn {
  return listenPresentationMessage<T>("command", (payload) =>
    callback({ payload }),
  );
}
