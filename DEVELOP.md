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

Tauri 命令层只负责调用转发，投屏逻辑放在 `karamas_core` 中。核心库保持独立，不依赖 Tauri 或前端状态。

前端使用 Vue 3 和 Vue Router，TypeScript 开启严格模式。源码内部导入统一使用 `@/`；导入源码目录外的文件时使用相对路径。共用样式放在全局，页面和组件的专用样式使用 `<style scoped>`。共享状态使用 Pinia，表单草稿、筛选条件和弹窗开关留在页面或组件内。

核心代码沿用现有格式，按具体任务修改相关文件，避免整库格式化。

## 本地运行

安装 Node.js 24 或更高版本、pnpm 11、Rust，并按 [Tauri 环境准备文档](https://v2.tauri.app/start/prerequisites/)安装对应平台的工具链。

在仓库根目录安装依赖并启动桌面应用：

```sh
pnpm install --frozen-lockfile
pnpm tauri dev
```

浏览器预览用于检查界面，投屏和登录需通过 Tauri 应用调试。

## 开发与检查

| 命令                               | 用途                           |
| ---------------------------------- | ------------------------------ |
| `pnpm dev`                         | 启动前端开发服务器             |
| `pnpm tauri dev`                   | 启动桌面应用调试               |
| `pnpm tauri build`                 | 构建桌面安装包                 |
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

运行 `pnpm version:set 0.2.0` 统一更新应用和核心库版本，支持 `0.2.0-beta.1` 等预发布版本。

发布 tag 使用 `v<版本号>`，`CHANGELOG.md` 中保留对应的 `[版本号]` 记录。

## 发布 Secrets

[发布工作流](.github/workflows/build-and-release.yml)需要以下 Secrets。

### Android（必填）

首次发布时生成 RSA 2048 位签名密钥，存入 JKS keystore；已发布的应用继续使用原有 keystore。

```bash
mkdir -p ~/.android
keytool -genkeypair -keystore ~/.android/karamas-release.jks -storetype JKS \
  -alias karamas -keyalg RSA -keysize 2048 -validity 10000
```

按提示设置密码和证书信息，`ANDROID_KEY_ALIAS` 填 `karamas`。macOS 可用 `base64 < ~/.android/karamas-release.jks | tr -d '\n' | pbcopy` 复制编码后的 keystore。

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

### 桌面自动更新

使用 Tauri signer 生成的 Minisign（Ed25519）密钥对，配置值填写其输出的 Base64 编码公钥和私钥完整内容。

在项目根目录执行，按提示设置私钥密码：

```bash
mkdir -p ~/.tauri
pnpm tauri signer generate -w ~/.tauri/karamas.key
```

公钥取 `karamas.key.pub`，私钥取 `karamas.key`，无需再次 Base64 编码。macOS 可用 `pbcopy < ~/.tauri/karamas.key.pub` 复制公钥，去掉 `.pub` 则复制私钥。后续版本继续使用同一对密钥。

| 名称                                 | 用途                                                      |
| ------------------------------------ | --------------------------------------------------------- |
| `TAURI_UPDATER_PUBLIC_KEY`           | Repository Variable 或同名 Secret，发布构建时注入更新公钥 |
| `TAURI_SIGNING_PRIVATE_KEY`          | Secret，Tauri 更新产物签名密钥                            |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Secret，密钥设有密码时填写                                |

普通开发和本地构建无需发布密钥。

## 更新源测试

更新地址统一读取 `plugins.updater.endpoints`：桌面默认值在 `tauri.conf.json`，Android 在 `tauri.android.conf.json` 覆盖。测试构建用 `--config <文件>` 覆盖同一字段；本地 HTTP 测试源还需在覆盖配置中设置 `plugins.updater.dangerousInsecureTransportProtocol: true`。

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

本地 release APK 需签名后安装；GitHub 发布工作流使用上述 Android Secrets 完成签名。

### Android 回归检查

先在模拟器验证安装、权限拒绝后重试、设备描述文件读取、投屏控制、退到桌面与锁屏后的自动切歌，以及断开和连接失败后的服务清理。再用同一 Wi-Fi 下的真机与电视验证自动搜索、手动连接、媒体代理拉流、播放与暂停、音量、进度和自动切歌。模拟器的网络环境与真实局域网不同，搜索空列表不代表电视离线。

同时检查弹窗和日志页的返回行为、主页连续返回退到桌面，以及日志保存、系统分享和取消操作。

### iOS

在 macOS 上准备好 iOS 工具链后，运行：

```sh
pnpm tauri ios init
pnpm tauri ios dev
```
