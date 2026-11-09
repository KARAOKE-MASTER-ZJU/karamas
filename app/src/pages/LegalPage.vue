<script setup lang="ts">
import { openUrl } from "@/services/platform";
import { ArrowUp } from "@lucide/vue";
import { onActivated, onDeactivated, ref } from "vue";
import { projectLicense, thirdPartyNotices } from "virtual:legal-documents";
import { useOperationNotice } from "@/composables/useOperationNotice";
import { errorMessage, isTauri } from "@/services/casting";

const props = defineProps<{ document: "license" | "third-party" }>();
const { notify } = useOperationNotice();
const showBackToTop = ref(false);
let viewport: HTMLElement | null = null;

function updateBackToTop() {
  showBackToTop.value = (viewport?.scrollTop ?? 0) > 320;
}

onActivated(() => {
  if (props.document !== "third-party") return;
  viewport = document.getElementById("main-content");
  viewport?.addEventListener("scroll", updateBackToTop, { passive: true });
  updateBackToTop();
});
onDeactivated(() => {
  viewport?.removeEventListener("scroll", updateBackToTop);
  viewport = null;
});

function backToTop() {
  document.getElementById("main-content")?.scrollTo({ top: 0 });
}

async function openLink(event: MouseEvent) {
  const anchor =
    event.target instanceof Element ? event.target.closest("a") : null;
  const href = anchor?.getAttribute("href");
  if (!href) return;
  event.preventDefault();
  if (href.startsWith("#")) {
    document.getElementById(href.slice(1))?.scrollIntoView({ block: "start" });
  } else if (/^https?:\/\//i.test(href)) {
    try {
      if (isTauri()) await openUrl(href);
      else window.open(href, "_blank", "noopener,noreferrer");
    } catch (error) {
      notify(`打开链接失败：${errorMessage(error)}`, "error");
    }
  }
}
</script>

<template>
  <pre v-if="document === 'license'" class="project-license">{{
    projectLicense
  }}</pre>
  <!-- HTML 仅由构建时的仓库声明生成。 -->
  <!-- eslint-disable vue/no-v-html -->
  <article
    v-else
    class="third-party-notices"
    @click="openLink"
    v-html="thirdPartyNotices"
  />
  <!-- eslint-enable vue/no-v-html -->
  <button
    v-if="document === 'third-party' && showBackToTop"
    type="button"
    class="button secondary back-to-top"
    @click="backToTop"
  >
    <ArrowUp :size="16" aria-hidden="true" />
    返回顶部
  </button>
</template>

<style scoped>
.project-license,
.third-party-notices {
  font-size: 13px;
  line-height: 1.8;
  overflow-wrap: anywhere;
  user-select: text;
}
.third-party-notices {
  padding-bottom: calc(var(--control-height) + 24px);
}
.project-license,
.third-party-notices :deep(pre) {
  margin: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
.third-party-notices :deep(h1),
.third-party-notices :deep(h2),
.third-party-notices :deep(h3) {
  margin: 24px 0 12px;
  font-size: 16px;
  font-weight: 500;
}
.third-party-notices :deep(p) {
  margin: 12px 0;
}
.third-party-notices :deep(table) {
  display: block;
  max-width: 100%;
  overflow-x: auto;
  border-collapse: collapse;
  margin: 16px 0;
}
.third-party-notices :deep(th),
.third-party-notices :deep(td) {
  border: 1px solid var(--line);
  padding: 8px 12px;
  min-width: 160px;
  text-align: left;
}
.third-party-notices :deep(a) {
  color: var(--accent);
}
.third-party-notices :deep([id]) {
  scroll-margin-top: 20px;
}
.back-to-top {
  position: fixed;
  right: 24px;
  bottom: 24px;
}
@media (max-width: 640px) {
  .back-to-top {
    right: 20px;
    bottom: calc(var(--navigation-bar-height) + 16px);
  }
}
</style>
