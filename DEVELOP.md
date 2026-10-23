# 开发说明

## 开发约定

Tauri 命令层只负责调用转发，投屏逻辑放在 `crates/casting-core` 中。核心库保持独立，服务端可以单独依赖它，无须构建前端或 Tauri 应用。

前端源码内部导入统一使用 `@/`；导入源码目录外的文件时使用相对路径。共用样式放在全局，页面和组件的专用样式使用 `<style scoped>`。共享状态使用 Pinia，表单草稿、筛选条件和弹窗开关留在页面或组件内。

## Rust 核心

核心源码来自 `KARAOKE-MASTER-ZJU/ktv-casting` 的提交 `517839d5e47cbfd7780f191ea5381de85ea65952`，直接存放在本仓库中。

Cargo 包名为 `ktv-casting`，库名为 `ktv_casting_lib`，应用通过依赖别名 `casting-core` 引用。`rupnp` 补丁必须配置在 workspace 根目录，Cargo 不读取成员 crate 中的补丁配置。

核心代码沿用现有格式，按具体任务修改相关文件，避免整库格式化。

核心目前为单会话，仍保留 JNI 接口。服务端多实例复用前，需要调整状态归属和任务生命周期。

`engine` 负责组合设备协议、房间同步和媒体服务；`cast`、`playlist`、`media` 和 `sources` 不反向依赖引擎。内部调用使用所属模块的接口，`lib.rs` 只声明模块和导出宿主入口。单元测试跟随所属模块。

## 播放与暂停接口

界面根据按钮显示的动作调用独立的 `play` 或 `pause` Tauri 命令，分别转发到核心库的 `play_core()` 和 `pause_core()`。播放成功返回 `true`，暂停成功返回 `false`，前端据此更新 `playback.isPlaying`。命令失败时保留原状态。

两个接口始终向设备发送明确命令，不根据本地记录跳过操作，并与新曲播放共用 `playback_lock`。原有 `toggle_pause` / `toggle_pause_core()` 和前端 `togglePause()` 保留。播放状态轮询和自动切歌沿用原有逻辑。

## 检查命令

浏览器预览用于检查界面，投屏和登录需通过 `pnpm tauri dev` 启动。

| 命令                                   | 用途                           |
| -------------------------------------- | ------------------------------ |
| `pnpm lint`                            | 检查前端代码，警告也会导致失败 |
| `pnpm format:check`                    | 检查前端、配置和文档格式       |
| `pnpm check`                           | TypeScript 类型检查            |
| `pnpm build`                           | 类型检查并构建前端             |
| `cargo check --workspace`              | 检查 Rust 工作区               |
| `cargo test -p ktv-casting --lib`      | 运行核心库测试                 |
| `cargo fmt -p ktv-casting-app --check` | 检查 Tauri 应用层 Rust 格式    |

`pnpm lint:fix` 和 `pnpm format` 会修改文件。前端检查和格式化的范围见 `eslint.config.mjs` 和 `.prettierignore`。

## 移动端开发

准备好 Android 工具链后，首次初始化原生工程，再启动开发：

```sh
pnpm tauri android init
pnpm tauri android dev
```

在 macOS 上准备好 iOS 工具链后，运行：

```sh
pnpm tauri ios init
pnpm tauri ios dev
```

移动端需在初始化原生工程后单独验证。
