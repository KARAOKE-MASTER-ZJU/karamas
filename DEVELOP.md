# 开发说明

## 开发约定

Tauri 命令层只负责调用转发，投屏逻辑放在 `crates/casting-core` 中。核心库保持独立，服务端可以单独依赖它，无须构建前端或 Tauri 应用。

前端源码内部导入统一使用 `@/`；导入源码目录外的文件时使用相对路径。共用样式放在全局，页面和组件的专用样式使用 `<style scoped>`。共享状态使用 Pinia，表单草稿、筛选条件和弹窗开关留在页面或组件内。

## Rust 核心

核心源码来自 `KARAOKE-MASTER-ZJU/ktv-casting` 的提交 `517839d5e47cbfd7780f191ea5381de85ea65952`，直接存放在本仓库中。

Cargo 包名为 `ktv-casting`，库名为 `ktv_casting_lib`，应用通过依赖别名 `casting-core` 引用。`rupnp` 补丁必须配置在 workspace 根目录，Cargo 不读取成员 crate 中的补丁配置。

核心代码沿用现有格式，按具体任务修改相关文件，避免整库格式化。

核心目前为单会话，仍保留原 Android 项目使用的 JNI 接口，其导出符号和类路径沿用旧包名 `zju.bangdream.ktv.casting`。当前 Tauri 应用通过命令桥接调用核心库。服务端多实例复用前，需要调整状态归属和任务生命周期。

`engine` 负责组合设备协议、房间同步和媒体服务；`cast`、`playlist`、`media` 和 `sources` 不反向依赖引擎。内部调用使用所属模块的接口，`lib.rs` 只声明模块和导出宿主入口。单元测试跟随所属模块。

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

### Android 构建

Android 原生工程保存在 `src-tauri/gen/android`，与应用源码一起维护。修改原生代码后不要重新运行 `android init`，以免覆盖接入代码。

安装 Android Studio、JDK 21 或更高版本、Android SDK 37 和 NDK。设置 `ANDROID_HOME` 指向 SDK；使用特定 NDK 时设置 `NDK_HOME`，使用特定 JDK 时设置 `JAVA_HOME`。macOS 上未设置 `JAVA_HOME` 时，启动脚本尝试使用 Android Studio 自带的 JDK。

启动开发或构建 arm64 调试 APK：

```sh
pnpm android dev
pnpm android build --debug --target aarch64 --apk
```

使用 `pnpm android` 构建，确保 QuickJS bindgen 使用 NDK 的目标头文件。相关配置见 `scripts/android.mjs`。首次构建需要下载 Gradle 和 Maven 依赖。

Android 图标统一维护在 `src-tauri/icons/android`，原生工程直接引用此目录。

APK 位于 `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`。可用 `adb install -r` 安装到调试设备。当前 applicationId 为 `zju.idol.karamas`，可与原 Android 项目并存，应用数据各自保存。

### Android 网络与运行限制

Android 17 及以上按需申请本地网络访问权限 `ACCESS_LOCAL_NETWORK`。DLNA 使用 HTTP，调试与 Release 构建均需允许明文流量。

当前 Android 适配以前台运行为前提，后台持续投屏尚未接入。

网络状态排查查看 `ANDROID_NETWORK` 日志，设备发现查看 `DLNA_DISCOVERY` 和 `DLNA_SSDP` 日志。

### Android 回归检查

先在模拟器验证安装、权限拒绝后重试、设备描述文件读取和投屏控制。再用同一 Wi-Fi 下的真机与电视验证自动搜索、手动连接、媒体代理拉流、播放与暂停、音量、进度和自动切歌。模拟器的网络环境与真实局域网不同，搜索空列表不代表电视离线。

### iOS

在 macOS 上准备好 iOS 工具链后，运行：

```sh
pnpm tauri ios init
pnpm tauri ios dev
```
