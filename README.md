# KTV Casting App

基于 Tauri 2、TypeScript 和 Rust 的跨平台投屏应用。

已提供设备连接、播放控制、设置和日志界面，支持保存房间配置、切换主题和导出界面日志。实际房间操作、设备连接与播放控制尚未开放。

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

工程结构、检查命令和移动端接入说明见 [开发文档](DEVELOP.md)。
