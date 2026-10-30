# 发布说明

下文命令均在仓库根目录执行。

## 应用版本

运行 `pnpm version:set 0.2.0` 统一更新应用和核心库版本。

发布 tag 使用 `v<版本号>`，[`CHANGELOG.md`](../CHANGELOG.md) 中保留对应的 `[版本号]` 记录。

> [!IMPORTANT]
> **发布Release之前一定要在[Unreleased]下加上[x.x.x]的二级标题，否则会发布失败。**

## 发布 Secrets

[发布工作流](../.github/workflows/build-and-release.yml)需要以下 Secrets。

### Android（必填）

首次发布时生成 RSA 2048 位签名密钥，存入 JKS keystore；已发布的应用继续使用原有 keystore。

```bash
mkdir -p ~/.android
keytool -genkeypair -keystore ~/.android/karamas-release.jks -storetype JKS \
  -alias karamas -keyalg RSA -keysize 2048 -validity 10000
```

按提示设置密码和证书信息。macOS 可用 `base64 < ~/.android/karamas-release.jks | tr -d '\n' | pbcopy` 复制编码后的 keystore。

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

公钥取 `karamas.key.pub`，私钥取 `karamas.key`。macOS 可用 `pbcopy < ~/.tauri/karamas.key.pub` 复制公钥，去掉 `.pub` 则复制私钥。后续版本继续使用同一对密钥。

| 名称                                 | 用途                                                      |
| ------------------------------------ | --------------------------------------------------------- |
| `TAURI_UPDATER_PUBLIC_KEY`           | Repository Variable 或同名 Secret，发布构建时注入更新公钥 |
| `TAURI_SIGNING_PRIVATE_KEY`          | Secret，Tauri 更新产物签名密钥                            |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Secret，密钥设有密码时填写                                |

普通开发和本地构建无需发布密钥。

## 更新源测试

更新地址通过 `plugins.updater.endpoints` 配置，默认配置文件位于：

- 桌面：[`app/src-tauri/tauri.conf.json`](../app/src-tauri/tauri.conf.json)。
- Android：[`app/src-tauri/tauri.android.conf.json`](../app/src-tauri/tauri.android.conf.json)，覆盖桌面配置中的更新地址。

测试时另建配置文件，通过构建参数 `--config <配置文件路径>` 覆盖该字段。使用本地 HTTP 测试源时，还需在该文件中设置 `plugins.updater.dangerousInsecureTransportProtocol: true`。
