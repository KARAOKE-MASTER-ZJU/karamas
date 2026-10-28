# KARAM@S

基于 Tauri 2、Vue 3、TypeScript 和 Rust 的跨平台投屏应用。

支持创建和加入点歌房间、本机播放、DLNA 设备搜索与投屏，以及哔哩哔哩小电视云投屏。可以控制播放、切歌、进度和音量，选择清晰度与分 P，并查看和导出日志。

## 本地运行

安装 Node.js 24 或更高版本、pnpm 11、Rust，并按 [Tauri 环境准备文档](https://v2.tauri.app/start/prerequisites/)安装对应平台的工具链。

在仓库根目录安装依赖并启动桌面应用：

```sh
pnpm install --frozen-lockfile
pnpm tauri dev
```

仅预览界面时，启动前端开发服务器；投屏和登录需要在 Tauri 应用中使用：

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

DLNA 投屏需要手机与电视连接同一网络。

Android 支持后台投屏，退到桌面或锁屏后仍会同步歌单和自动切歌。

## 应用更新

正式发布版启动后自动检查更新，也可以在“设置 → 关于”中手动检查。下载中可以取消，完成后点击“安装并重启”。Android 会打开系统安装器，首次安装更新时按提示允许安装此来源的应用。

工程结构、检查命令和移动端接入说明见 [开发文档](DEVELOP.md)。
