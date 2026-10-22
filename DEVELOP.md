# 开发说明

## 开发约定

Tauri 命令层只负责调用转发，投屏逻辑放在 `crates/casting-core` 中。核心库保持独立，服务端可以单独依赖它，无须构建前端或 Tauri 应用。

前端源码内部导入统一使用 `@/`；导入源码目录外的文件时使用相对路径。共用样式放在全局，页面和组件的专用样式使用 `<style scoped>`。共享状态使用 Pinia，表单草稿、筛选条件和弹窗开关留在页面或组件内。

## Rust 核心

核心源码来自 `KARAOKE-MASTER-ZJU/ktv-casting` 的提交 `517839d5e47cbfd7780f191ea5381de85ea65952`，直接存放在本仓库中。

Cargo 包名为 `ktv-casting`，库名为 `ktv_casting_lib`，应用通过依赖别名 `casting-core` 引用。`rupnp` 补丁必须配置在 workspace 根目录，Cargo 不读取成员 crate 中的补丁配置。

核心代码沿用现有格式，按具体任务修改相关文件，避免整库格式化。

`engine` 负责组合设备协议、房间同步和媒体服务；`cast`、`playlist`、`media` 和 `sources` 不反向依赖引擎。内部调用使用所属模块的接口，`lib.rs` 只声明模块和导出宿主入口。JNI 的登录展示状态留在 `android`，CLI 留在二进制入口中。单元测试跟随所属模块，迁移文件时保留原有测试内容。

## Tauri 接入

`src-tauri/src/commands` 按设备、会话、播放控制和登录划分命令。命令复用 Tauri 运行时，核心库继续持有单会话状态；应用退出时重置引擎并取消扫码任务。

通过 `invoke` 传入结构化参数，不需要手动序列化 JSON。命令参数和适配层返回字段使用 camelCase，例如启动 DLNA 会话：

```ts
await invoke("start_casting", {
  serverUrl: "https://你的点歌台网址",
  roomId: "房间号",
  target: { kind: "dlna", location: "http://设备地址/description.xml" },
});
```

房间应由调用方提前创建或加入。B站目标参数为 `{ kind: "bilibili", session, deviceBuvid }`。`poll_playback` 由界面定期调用，同时驱动自动切歌；未连接时返回 null，当前进度不可信时进度字段为 null。

`login_bilibili` 接收名为 `onQr` 的 `Channel<string>`，通过通道返回二维码地址，成功后返回 `BilibiliSession`。会话沿用核心库字段 `access_token`、`mid`、`expires_at`；登录和启动投屏都不自动保存会话，保存、读取和清除分别使用 `save_bilibili_session`、`load_bilibili_session`、`clear_bilibili_session`。关闭扫码界面时调用 `cancel_bilibili_login`。

命令错误返回 `{ code, message }`。Rust 日志通过 `casting://log` 事件发送 `{ level, target, message }`，与界面提示条独立。

## 检查命令

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

## 接入状态

Tauri 命令已连接 Rust 核心，前端页面仍保留占位交互，尚未调用这些命令或订阅 Rust 日志。房间配置和主题保存在本机。

核心仍使用原有的单会话状态和 JNI 接口。服务端多实例复用前，需要调整状态归属和任务生命周期。
