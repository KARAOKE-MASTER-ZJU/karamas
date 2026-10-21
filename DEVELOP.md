# 开发说明

## 工程结构

- `src/`：前端入口和样式。暂用 Vite 和原生 TypeScript，界面框架待确定。
- `src-tauri/`：Tauri 应用启动、配置，以及后续的前端命令桥接。
- `crates/casting-core/`：独立于应用框架的 Rust 投屏核心，包含源码、测试和示例。
- 根目录的 `Cargo.toml`：声明 workspace 成员，统一管理 Rust 依赖锁文件和构建目录。

Tauri 命令层负责调用转发，投屏逻辑放在核心库中。核心库不依赖 Tauri，服务端可以单独依赖这个 crate，无须构建前端或 Tauri 应用。

## 核心源码来源

核心源码来自 `KARAOKE-MASTER-ZJU/ktv-casting` 的提交 `517839d5e47cbfd7780f191ea5381de85ea65952`，直接存放在本仓库中，不使用 Git 子模块。

本次搭建保留原 Cargo 包名 `ktv-casting` 和库名 `ktv_casting_lib`，应用通过依赖别名 `casting-core` 引用它。

Rust 源码、测试和示例均保持原样。现有 `rupnp` 补丁配置移至 workspace 根目录，这是 Cargo 对 workspace 补丁配置的要求。后续核心接入改动应与工程骨架分开处理。

## 检查和构建

```sh
pnpm check
pnpm build
cargo check --workspace
cargo test -p ktv-casting --lib
pnpm tauri build
```

检查 Tauri 应用层 Rust 代码的格式：

```sh
cargo fmt -p ktv-casting-app --check
```

核心代码沿用现有格式，按具体任务修改相关文件。

## 移动端工程

准备好 Android 工具链后，初始化原生工程并启动开发：

```sh
pnpm tauri android init
pnpm tauri android dev
```

在 macOS 上准备好 iOS 工具链后，运行：

```sh
pnpm tauri ios init
pnpm tauri ios dev
```

这些命令用于初始化和启动应用外壳。核心的移动端依赖、旧 JNI 桥接和平台服务仍需接入并验证。

## 待接入内容

Tauri 应用已声明对核心的 Cargo 依赖，前端尚未调用投屏功能。投屏命令、登录流程、多实例管理、服务端接口和界面框架均待接入。

核心目前仍采用原有的单会话状态和 JNI 接口。后续需要服务端多实例复用时，再单独调整状态归属和任务生命周期。
