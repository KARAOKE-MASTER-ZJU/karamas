import { watch, type Ref } from "vue";
import { useOperationNotice } from "@/composables/useOperationNotice";
import { errorMessage } from "@/services/casting";

export function useLocalPlayerShortcuts(
  video: Ref<HTMLVideoElement | null>,
  enabled: Ref<boolean>,
) {
  const { notify } = useOperationNotice();
  watch(
    [video, enabled],
    ([element, active], _, onCleanup) => {
      if (!element || !active) return;
      const document = element.ownerDocument;
      const keydown = (event: KeyboardEvent) => {
        if (
          event.defaultPrevented ||
          event.isComposing ||
          event.altKey ||
          event.ctrlKey ||
          event.metaKey ||
          event.shiftKey ||
          document.querySelector("dialog[open]")
        )
          return;
        const target = event.target;
        if (
          target instanceof HTMLElement &&
          (target.isContentEditable ||
            target.closest(
              'input, textarea, select, button, a[href], [role="button"], [role="slider"], [role="textbox"]',
            ))
        )
          return;
        if (
          ![" ", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(
            event.key,
          )
        )
          return;

        // 在原生控件处理之前接管，避免同一次按键执行两次或滚动页面。
        event.preventDefault();
        event.stopPropagation();
        switch (event.key) {
          case " ":
            if (event.repeat) return;
            if (element.paused)
              void element.play().catch((cause: unknown) => {
                notify(errorMessage(cause), "error");
              });
            else element.pause();
            break;
          case "ArrowLeft":
          case "ArrowRight":
            if (!Number.isFinite(element.duration)) return;
            element.currentTime = Math.max(
              0,
              Math.min(
                element.duration,
                element.currentTime + (event.key === "ArrowRight" ? 5 : -5),
              ),
            );
            break;
          case "ArrowUp":
          case "ArrowDown": {
            const volume = element.muted ? 0 : Math.round(element.volume * 100);
            element.volume =
              Math.max(
                0,
                Math.min(100, volume + (event.key === "ArrowUp" ? 5 : -5)),
              ) / 100;
            element.muted = false;
            break;
          }
        }
      };
      document.addEventListener("keydown", keydown, true);
      onCleanup(() => document.removeEventListener("keydown", keydown, true));
    },
    { immediate: true, flush: "post" },
  );
}
