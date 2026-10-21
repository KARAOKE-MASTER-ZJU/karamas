# KTV Casting App

基于 Tauri 2、TypeScript 和 Rust 的跨平台投屏应用。

目前处于工程搭建阶段，桌面应用可启动并显示基础页面，投屏功能尚未接入界面。

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
