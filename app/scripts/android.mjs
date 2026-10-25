import process from "node:process";
import { existsSync, readdirSync } from "node:fs";
import { homedir, platform } from "node:os";
import { join } from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath, URL } from "node:url";
import { spawnSync } from "node:child_process";

// 为 Tauri 和 QuickJS bindgen 共用同一套 Android NDK 头文件。
const env = { ...process.env };
const os = platform();
const sdk =
  env.ANDROID_HOME ||
  env.ANDROID_SDK_ROOT ||
  join(
    homedir(),
    os === "darwin"
      ? "Library/Android/sdk"
      : os === "win32"
        ? "AppData/Local/Android/Sdk"
        : "Android/Sdk",
  );
if (!existsSync(sdk))
  throw new Error("未找到 Android SDK，请设置 ANDROID_HOME。");
env.ANDROID_HOME = sdk;
const ndkRoot = join(sdk, "ndk");
const versions = existsSync(ndkRoot)
  ? readdirSync(ndkRoot)
      .filter((name) => existsSync(join(ndkRoot, name, "source.properties")))
      .sort((a, b) => b.localeCompare(a, undefined, { numeric: true }))
  : [];
const ndk =
  env.NDK_HOME ||
  env.ANDROID_NDK_HOME ||
  (versions[0] && join(ndkRoot, versions[0]));
if (!ndk) throw new Error("未找到 Android NDK，请设置 NDK_HOME。");
env.NDK_HOME = ndk;
const prebuilt = join(ndk, "toolchains/llvm/prebuilt");
const host = readdirSync(prebuilt).find((name) =>
  name.startsWith(
    os === "darwin" ? "darwin-" : os === "win32" ? "windows-" : "linux-",
  ),
);
if (!host) throw new Error("NDK 中没有适合当前系统的 LLVM 工具链。");
const sysroot = join(prebuilt, host, "sysroot");
for (const [target, headers] of Object.entries({
  aarch64_linux_android: "aarch64-linux-android",
  armv7_linux_androideabi: "arm-linux-androideabi",
  i686_linux_android: "i686-linux-android",
  x86_64_linux_android: "x86_64-linux-android",
})) {
  const key = `BINDGEN_EXTRA_CLANG_ARGS_${target}`;
  env[key] ??=
    `--target=${headers}24 --sysroot="${sysroot}" -I"${join(sysroot, "usr/include", headers)}"`;
}
if (!env.JAVA_HOME && os === "darwin") {
  const bundledJava =
    "/Applications/Android Studio.app/Contents/jbr/Contents/Home";
  if (existsSync(bundledJava)) env.JAVA_HOME = bundledJava;
}
// 直接启动已安装的 CLI，让 Gradle 子进程也继承 bindgen 配置。
const cli = createRequire(import.meta.url).resolve("@tauri-apps/cli/tauri.js");
const result = spawnSync(
  process.execPath,
  [cli, "android", ...process.argv.slice(2)],
  {
    cwd: fileURLToPath(new URL("..", import.meta.url)),
    env,
    stdio: "inherit",
  },
);
if (result.error) throw result.error;
process.exit(result.status ?? 1);
