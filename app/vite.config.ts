import { fileURLToPath, URL } from "node:url";
import { readFileSync } from "node:fs";
import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { Marked } from "marked";

const host = process.env.TAURI_DEV_HOST;
const legalModule = "virtual:legal-documents";
const markdown = new Marked({
  renderer: {
    heading({ tokens, depth }) {
      const text = this.parser.parseInline(tokens);
      const id = /^(license-\d+|notice-\d+-full)$/.test(text)
        ? ` id="${text}"`
        : "";
      return `<h${depth}${id}>${text}</h${depth}>\n`;
    },
  },
});

export default defineConfig({
  plugins: [
    vue(),
    {
      name: "legal-documents",
      resolveId(id) {
        if (id === legalModule) return `\0${legalModule}`;
      },
      load(id) {
        if (id !== `\0${legalModule}`) return;
        const licensePath = fileURLToPath(
          new URL("../LICENSE", import.meta.url),
        );
        const noticesPath = fileURLToPath(
          new URL("../THIRD_PARTY_NOTICES.md", import.meta.url),
        );
        this.addWatchFile(licensePath);
        this.addWatchFile(noticesPath);
        return `export const projectLicense = ${JSON.stringify(readFileSync(licensePath, "utf8"))};
export const thirdPartyNotices = ${JSON.stringify(markdown.parse(readFileSync(noticesPath, "utf8")))};`;
      },
    },
  ],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: {
      ignored: ["**/src-tauri/**", "**/karamas_core/**", "**/target/**"],
    },
  },
  build: { target: "es2021" },
});
