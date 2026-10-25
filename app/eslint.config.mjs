import js from "@eslint/js";
import { globalIgnores } from "eslint/config";
import pluginVue from "eslint-plugin-vue";
import { withVueTs, vueTsConfigs } from "@vue/eslint-config-typescript";
import eslintConfigPrettier from "eslint-config-prettier/flat";
import globals from "globals";

export default withVueTs(
  {
    name: "app/files-to-lint",
    files: ["**/*.{vue,ts,mts,tsx,js,mjs,cjs}"],
  },
  globalIgnores([
    "**/dist/**",
    "**/coverage/**",
    "**/target/**",
    "src-tauri/**",
  ]),
  js.configs.recommended,
  pluginVue.configs["flat/recommended"],
  vueTsConfigs.recommended,
  {
    name: "app/browser-globals",
    files: ["src/**/*.{ts,vue}"],
    languageOptions: { globals: globals.browser },
  },
  {
    name: "app/node-globals",
    files: ["*.{js,mjs,cjs,ts,mts,cts}"],
    languageOptions: { globals: globals.node },
  },
  eslintConfigPrettier,
);
