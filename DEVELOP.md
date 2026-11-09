# 开发说明

## 工程结构

`app/` 是完整的 Tauri 应用，包含前端源码、原生接入、依赖和构建配置；`karamas_core/` 是独立的 Rust 核心库。

```text
app/
├── src/                 # Vue 前端
├── src-tauri/           # Tauri 命令、平台接入和原生工程
├── public/              # 前端静态资源
├── scripts/             # 应用构建脚本
├── package.json         # 应用依赖和命令
└── vite.config.ts       # 前端构建配置
karamas_core/             # 独立投屏核心
Cargo.toml               # Rust 工作区
package.json             # 根目录命令入口和格式化工具
pnpm-workspace.yaml      # pnpm 工作区
```

下文命令均在仓库根目录执行。前端构建产物位于 `app/dist/`，Rust 构建产物位于 `target/`。

## 开发约定

Tauri 命令层只负责调用转发，投屏逻辑放在 `karamas_core` 中。

前端使用 Vue 3 和 Vue Router，TypeScript 开启严格模式。源码内部导入统一使用 `@/`；导入源码目录外的文件时使用相对路径。

共用样式放在全局，页面和组件的专用样式使用 `<style scoped>`。

## 本地运行

安装 Node.js 24 或更高版本、pnpm 11、Rust，并按 [Tauri 环境准备文档](https://v2.tauri.app/start/prerequisites/)安装对应平台的工具链。

在仓库根目录安装依赖并启动桌面应用：

```sh
pnpm install --frozen-lockfile
pnpm tauri dev
```

## 开发与检查

| 命令                               | 用途                      |
| ---------------------------------- | ------------------------- |
| `pnpm dev`                         | 启动前端开发服务器        |
| `pnpm tauri dev`                   | 启动桌面应用调试          |
| `pnpm tauri build`                 | 构建桌面安装包            |
| `pnpm check:all`                   | 运行 lint、格式和类型检查 |
| `pnpm fix:all`                     | 自动修复 lint 和格式      |
| `pnpm build`                       | 类型检查并构建前端        |
| `cargo check --workspace`          | 检查 Rust 工作区编译      |
| `cargo test -p karamas_core --lib` | 运行核心库测试            |
| `cargo test -p karamas --lib`      | 运行应用层测试            |

浏览器预览用于检查界面，投屏和登录需通过 Tauri 应用调试。

提交前自动运行 `pnpm format:check`，提醒遗漏的格式问题。本地可运行 `pnpm check:all` 检查 lint、格式和类型，Rust 编译和测试需另外运行表中的对应命令。完整检查由 CI 执行。

CI 配置见 [GitHub Actions 工作流](.github/workflows/ci.yml)。

## 发布

版本管理、签名密钥和更新源测试见[发布说明](docs/RELEASE.md)。

## 日志

在设置页调整日志记录级别，通过“运行日志”查看和导出当前记录。历史文件保存在 Tauri 的 `app_log_dir`，Rust panic 另写入 `panics.log`。

## 移动端开发

### Android 构建

Android 原生工程保存在 `app/src-tauri/gen/android`，与应用源码一起维护。修改原生代码后不要重新运行 `android init`，以免覆盖接入代码。

安装 Android Studio、JDK 21 或更高版本、Android SDK 37 和 NDK。设置 `ANDROID_HOME` 指向 SDK；使用特定 NDK 时设置 `NDK_HOME`，使用特定 JDK 时设置 `JAVA_HOME`。macOS 上未设置 `JAVA_HOME` 时，启动脚本尝试使用 Android Studio 自带的 JDK。

启动开发或构建 arm64 调试 APK：

```sh
pnpm android dev
pnpm android build --debug --target aarch64 --apk
```

首次构建需要下载 Gradle 和 Maven 依赖。APK 输出到 `app/src-tauri/gen/android/app/build/outputs/apk/`，具体路径见构建输出。单独构建的调试 APK 可用 `adb install -r <APK 路径>` 安装。

构建 arm64 release APK：

```sh
pnpm android build --target aarch64 --apk
```

GitHub 发布的 APK 启用 `.so` 压缩，本地构建默认不压缩。本地构建压缩版 APK：

```sh
ORG_GRADLE_PROJECT_karamasCompressNativeLibs=true pnpm android build --target aarch64 --apk
```

本地 release APK 需签名后安装；GitHub 发布工作流使用[发布说明中的 Android Secrets](docs/RELEASE.md#android必填)完成签名。

### iOS

在 macOS 上准备好 iOS 工具链后，运行：

```sh
pnpm tauri ios init
pnpm tauri ios dev
```
