# KTV Casting App

基于 Tauri 2、TypeScript 和 Rust 的跨平台投屏应用。

支持创建和加入点歌房间、搜索与连接 DLNA 设备，以及哔哩哔哩小电视云投屏。可以控制播放、切歌、进度和音量，选择清晰度与分 P，并查看和导出日志。

## 本地运行

安装 Node.js 24 或更高版本、pnpm 11、Rust，并按 [Tauri 环境准备文档](https://v2.tauri.app/start/prerequisites/)安装对应平台的工具链。

安装依赖并启动桌面应用：

```sh
pnpm install --frozen-lockfile
pnpm tauri dev
```

仅启动前端开发服务器：

```sh
pnpm dev
```

## 构建

```sh
pnpm tauri build
```

## Android

安装 Android Studio、JDK、Android SDK 和 NDK 后，在连接的手机或已启动的模拟器上运行：

```sh
pnpm android dev
```

构建 arm64 调试 APK：

```sh
pnpm android build --debug --target aarch64 --apk
```

Android 版目前适配应用保持前台时的设备发现与投屏。搜索和连接局域网设备时，请允许本地网络访问，并让手机与电视连接同一网络。后台持续投屏尚未接入。

工程结构、检查命令和移动端接入说明见 [开发文档](DEVELOP.md)。
