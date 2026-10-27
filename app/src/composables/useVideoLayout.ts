import { watch, type Ref } from "vue";

export function useVideoLayout(
  container: Ref<HTMLElement | null>,
  viewport: Ref<HTMLElement | null>,
  content: Ref<HTMLElement | null>,
  active: Ref<boolean>,
) {
  watch(
    [container, viewport, content, active],
    ([target, viewportElement, contentElement, visible], _, onCleanup) => {
      if (!visible || !target || !viewportElement || !contentElement) return;
      let frame = 0;
      const fitVideo = () => {
        cancelAnimationFrame(frame);
        frame = requestAnimationFrame(() => {
          // 相对内容容器计算偏移，滚动不会改变尺寸；下方控件仍可滚动查看。
          const videoTop =
            target.getBoundingClientRect().top -
            contentElement.getBoundingClientRect().top;
          const available = Math.max(
            1,
            Math.floor(viewportElement.clientHeight - videoTop - 16),
          );
          const value = `${available}px`;
          if (target.style.getPropertyValue("--local-video-max-height") !== value)
            target.style.setProperty("--local-video-max-height", value);
        });
      };
      const observer = new ResizeObserver(fitVideo);
      observer.observe(contentElement);
      observer.observe(viewportElement);
      fitVideo();
      onCleanup(() => {
        observer.disconnect();
        cancelAnimationFrame(frame);
      });
    },
    { immediate: true, flush: "post" },
  );
}
