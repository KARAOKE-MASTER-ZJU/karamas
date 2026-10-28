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

根目录的 `pnpm dev`、`pnpm build`、`pnpm check`、`pnpm lint`、`pnpm tauri` 和 `pnpm android` 转发到 `app/`；也可以进入 `app/` 运行这些命令。`pnpm format` 和 `pnpm format:check` 在根目录执行，覆盖仓库文档和应用文件。依赖锁文件保留在根目录。

Rust 共用根目录的 `Cargo.lock` 和 `target/`；前端构建产物位于 `app/dist/`。

## 开发约定

Tauri 命令层只负责调用转发，投屏逻辑放在 `karamas_core` 中。核心库保持独立，服务端可以单独依赖它，无须构建前端或 Tauri 应用。

开发和打包后的可执行文件名均为 `karamas`，应用显示名为 `KARAM@S`。macOS 开发模式的 Dock 使用可执行文件名，打包后的应用使用产品名。

前端使用 Vue 3 和 Vue Router，TypeScript 开启严格模式。源码内部导入统一使用 `@/`；导入源码目录外的文件时使用相对路径。共用样式放在全局，页面和组件的专用样式使用 `<style scoped>`。共享状态使用 Pinia，表单草稿、筛选条件和弹窗开关留在页面或组件内。

## Rust 核心

核心源码来自 `KARAOKE-MASTER-ZJU/ktv-casting` 的提交 `517839d5e47cbfd7780f191ea5381de85ea65952`，直接存放在本仓库中。

Cargo 包名、Rust 库名和应用中的依赖名统一为 `karamas_core`。`rupnp` 补丁必须配置在 workspace 根目录，Cargo 不读取成员 crate 中的补丁配置。

核心代码沿用现有格式，按具体任务修改相关文件，避免整库格式化。

核心目前为单会话，通过 Rust 接口供宿主调用。当前桌面端和 Android 端均通过 Tauri 命令桥接接入，运行时和日志由宿主初始化。服务端多实例复用前，需要调整状态归属和任务生命周期。

`engine` 负责组合设备协议、房间同步和媒体服务；`cast`、`playlist`、`media` 和 `sources` 不反向依赖引擎。内部调用使用所属模块的接口，`lib.rs` 只声明模块和导出宿主入口。单元测试跟随所属模块。

进度查询和自动切歌由核心会话任务驱动，前端定时读取缓存来刷新界面。前端停止刷新不影响核心任务；Android 通过前台服务维持后台会话。

房间同步、进度查询、媒体代理和小电视状态回传的流程见[核心库运行架构](karamas_core/DEVELOP.md)。

## 检查命令

浏览器预览用于检查界面，投屏和登录需通过 `pnpm tauri dev` 启动。

| 命令                               | 用途                           |
| ---------------------------------- | ------------------------------ |
| `pnpm check:all`                   | 运行全部 lint、格式和类型检查  |
| `pnpm fix:all`                     | 自动修复 lint 和格式           |
| `pnpm lint`                        | 检查前端代码，警告也会导致失败 |
| `pnpm format:check`                | 检查前端、配置和文档格式       |
| `pnpm check`                       | TypeScript 类型检查            |
| `pnpm build`                       | 类型检查并构建前端             |
| `cargo check --workspace`          | 检查 Rust 工作区               |
| `cargo test -p karamas_core --lib` | 运行核心库测试                 |
| `cargo fmt -p karamas --check`     | 检查 Tauri 应用层 Rust 格式    |

`pnpm lint:fix` 和 `pnpm format` 会修改文件。前端检查和格式化的范围见 `app/eslint.config.mjs` 和 `.prettierignore`。

提交前自动运行 `pnpm format:check`，提醒遗漏的格式问题。完整检查由 CI 执行，本地也可以运行 `pnpm check:all`。

CI 配置见 [GitHub Actions 工作流](.github/workflows/ci.yml)。

## 应用版本

运行 `pnpm version:set 0.2.0` 统一更新应用和核心库版本，支持 `0.2.0-beta.1` 等预发布版本。命令同步修改 `app/package.json`、`app/src-tauri/Cargo.toml`、`karamas_core/Cargo.toml` 和 `Cargo.lock`。

发布 tag 使用 `v<版本号>`，`CHANGELOG.md` 中保留对应的 `[版本号]` 记录。

## 发布 Secrets

[发布工作流](.github/workflows/build-and-release.yml)需要以下 Secrets。

### Android（必填）

四个 ABI 共用以下配置。

| Secret                      | 填写内容                         |
| --------------------------- | -------------------------------- |
| `ANDROID_KEYSTORE_BASE64`   | 发布 keystore 文件的 Base64 内容 |
| `ANDROID_KEYSTORE_PASSWORD` | keystore 的密码                  |
| `ANDROID_KEY_ALIAS`         | keystore 中用于签名的密钥别名    |
| `ANDROID_KEY_PASSWORD`      | 该密钥的密码                     |

### macOS（可选，启用时需填齐）

启用 Developer ID 签名和公证时需填齐以下六项；全部留空则使用 ad-hoc 签名，不做公证。

| Secret                       | 填写内容                                                           |
| ---------------------------- | ------------------------------------------------------------------ |
| `APPLE_CERTIFICATE`          | 包含私钥的 Developer ID Application 证书 `.p12` 文件的 Base64 内容 |
| `APPLE_CERTIFICATE_PASSWORD` | 导出 `.p12` 文件时设置的密码                                       |
| `APPLE_SIGNING_IDENTITY`     | 完整签名身份，例如 `Developer ID Application: Your Name (TEAMID)`  |
| `APPLE_ID`                   | 用于公证的 Apple 账号邮箱                                          |
| `APPLE_PASSWORD`             | Apple 账号的 App 专用密码（app-specific password）                 |
| `APPLE_TEAM_ID`              | Apple Developer 团队 ID                                            |

### GitHub 和其他平台

`GITHUB_TOKEN` 由 GitHub Actions 自动提供。

### 桌面自动更新

| 名称                                 | 用途                                                      |
| ------------------------------------ | --------------------------------------------------------- |
| `TAURI_UPDATER_PUBLIC_KEY`           | Repository Variable 或同名 Secret，发布构建时注入更新公钥 |
| `TAURI_SIGNING_PRIVATE_KEY`          | Secret，Tauri 更新产物签名密钥                            |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Secret，密钥设有密码时填写                                |

发布工作流通过临时配置启用更新产物，注入公钥。本地配置的公钥留空，普通开发和构建无需发布密钥。

## 自动更新

桌面使用 `tauri-plugin-updater`，Android 由 Rust 下载 APK、Kotlin 调用系统安装器。平台实现位于 `app/src-tauri/src/updater/`，界面状态由 `app/src/stores/updater.ts` 管理。

更新地址统一读取 `plugins.updater.endpoints`：桌面默认值在 `tauri.conf.json`，Android 在 `tauri.android.conf.json` 覆盖。测试构建用 `--config <文件>` 覆盖同一字段；本地 HTTP 测试源还需在覆盖配置中设置 `plugins.updater.dangerousInsecureTransportProtocol: true`。

各平台产物上传后，发布流程通过 `app/scripts/update-manifests.mjs` 生成并校验 `latest.json` 和 `android-update.json`，再公开 Release。应用只检查最新正式版，操作方式见 [README](README.md#应用更新)。

## 日志

`app/src-tauri/src/logging` 统一接收 Rust 和前端日志，默认记录 `info` 及更严重的日志；设置页可调整记录级别。内存保留最近 10,000 条记录，日志页的级别和关键词筛选只影响展示。清空操作只清除内存记录，保存和分享导出全部内存记录。

文件日志写入 Tauri 的 `app_log_dir`，单个文件达到 10 MiB 后轮转，保留 4 个归档文件；Rust panic 另写入 `panics.log`。文件日志与内存留存各自独立，日志页不读取历史文件。

保存使用系统文件对话框。系统分享在 Android、iOS 和 macOS 上开放，分享文件暂存于应用缓存目录的 `log-exports` 中。Windows 和 Linux 使用保存功能。相关实现位于 `app/src-tauri/src/logging/export.rs`。

## 移动端开发

### Android 构建

Android 原生工程保存在 `app/src-tauri/gen/android`，与应用源码一起维护。修改原生代码后不要重新运行 `android init`，以免覆盖接入代码。

安装 Android Studio、JDK 21 或更高版本、Android SDK 37 和 NDK。当前 `compileSdk` 和 `targetSdk` 均为 37，`minSdk` 为 24。设置 `ANDROID_HOME` 指向 SDK；使用特定 NDK 时设置 `NDK_HOME`，使用特定 JDK 时设置 `JAVA_HOME`。macOS 上未设置 `JAVA_HOME` 时，启动脚本尝试使用 Android Studio 自带的 JDK。

启动开发或构建 arm64 调试 APK：

```sh
pnpm android dev
pnpm android build --debug --target aarch64 --apk
```

使用 `pnpm android` 构建，确保 QuickJS bindgen 使用 NDK 的目标头文件。相关配置见 `app/scripts/android.mjs`。首次构建需要下载 Gradle 和 Maven 依赖。

### Release 体积优化

根目录的 `[profile.release]` 使用 `opt-level = "s"`、ThinLTO 和单个 codegen unit，并剥离符号表。panic 保持默认的 unwind 行为，继续保留 panic 日志和堆栈回溯。

构建 arm64 release APK：

```sh
pnpm android build --target aarch64 --apk
```

GitHub 发布的 APK 启用 `.so` 压缩，本地构建默认不压缩。本地构建压缩版 APK：

```sh
ORG_GRADLE_PROJECT_karamasCompressNativeLibs=true pnpm android build --target aarch64 --apk
```

两种构建均输出到 `app/src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk`，会覆盖上次产物。本地构建产物需签名后安装；GitHub 发布工作流使用上述 Android Secrets 完成签名。

### Android 图标与调试产物

Android 图标统一维护在 `app/src-tauri/icons/android`，原生工程直接引用此目录。

APK 位于 `app/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`。可用 `adb install -r` 安装到调试设备。当前 applicationId 为 `zju.idol.karamas`，可与原 Android 项目并存，应用数据各自保存。

### Android 网络与运行限制

Android 17 及以上按需申请本地网络访问权限 `ACCESS_LOCAL_NETWORK`。DLNA 使用 HTTP，调试与 Release 构建均需允许明文流量。

Android 投屏会话使用 `mediaPlayback` 类型的前台服务。服务在连接设备前启动，断开或连接失败时停止；返回桌面、切换应用和锁屏后，Rust 继续同步歌单、查询进度、自动切歌及提供媒体代理。服务仅在会话期间持有 CPU 唤醒锁，不申请多播锁。

Android 13 及以上在开始投屏时申请通知权限。拒绝后仍可投屏，但通知栏不显示投屏通知，前台服务仍可在系统的运行中应用列表中查看。本地网络和通知权限共用串行申请队列，同一权限的并发申请会合并。

原生接入分工：`CastingPlatformPlugin` 负责权限与服务启停桥接，`CastingService` 负责常驻通知和唤醒锁；播放与房间逻辑继续放在 Rust 核心。服务启动成功后才初始化核心会话。最终销毁 Activity 或从最近任务移除应用时停止服务；配置变化保留服务。进程终止后不自动重建会话，也不恢复旧房间。

网络状态排查查看 `ANDROID_NETWORK` 日志，设备发现查看 `DLNA_DISCOVERY` 和 `DLNA_SSDP` 日志，服务启停查看 `CASTING_SERVICE` 日志。

### Android 返回行为

`app/src/composables/useAndroidBack.ts` 接管返回键和返回手势。有弹窗时先关闭最上层弹窗，日志页返回设置页。在“连接设备”“播放控制”“设置”主页，第一次返回时提示，2 秒内再次返回则调用 `moveTaskToBack(true)` 退到桌面，投屏服务继续运行。切换页面或应用可见状态变化后，连续返回的计时会重置。

### Android 回归检查

先在模拟器验证安装、权限拒绝后重试、设备描述文件读取、投屏控制、退到桌面与锁屏后的自动切歌，以及断开和连接失败后的服务清理。再用同一 Wi-Fi 下的真机与电视验证自动搜索、手动连接、媒体代理拉流、播放与暂停、音量、进度和自动切歌。模拟器的网络环境与真实局域网不同，搜索空列表不代表电视离线。

同时检查弹窗和日志页的返回行为、主页连续返回退到桌面，以及日志保存、系统分享和取消操作。

### iOS

在 macOS 上准备好 iOS 工具链后，运行：

```sh
pnpm tauri ios init
pnpm tauri ios dev
```
