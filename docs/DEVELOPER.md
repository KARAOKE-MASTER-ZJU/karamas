# 开发者文档（ktv-casting）

本文面向要在本仓库上二次开发/调试的开发者。

---

## 项目概览

`ktv-casting` 是一个将 [ktv-song-web](https://github.com/KARAOKE-MASTER-ZJU/ktv-song-web) 点歌台的歌曲投屏到 **DLNA 设备** 或 **哔哩哔哩小电视** 的工具与核心引擎。

基本工作流程：

1. 输入房间链接（例如 `https://ktv.example.com/102`）
2. 程序解析出 `base_url` 与 `room_id`
3. 选择投屏模式：
   - **DLNA 模式**：通过 UPnP/SSDP 发现局域网内的 MediaRenderer，通过 SOAP (AVTransport) 控制播放
   - **Bilibili 模式**：扫码登录 B 站账号，通过 B 站云投屏 API 控制已配对的 TV 端设备
4. 启动一个本地 HTTP 服务（默认 `0.0.0.0:8080`）提供媒体代理/转发（DLNA 模式）
5. 后台通过 WebSocket 或轮询同步歌单、检测播放进度、自动切歌

---

## 核心模块

| 模块 | 路径 | 说明 |
|------|------|------|
| **库入口** | `src/lib.rs` | 引擎初始化、设备连接、房间连接、核心 API（`start_engine_core`、`get_current_progress`、`toggle_pause_core` 等） |
| **CLI 入口** | `src/main.rs` | 交互式命令行界面（仅启用 `cli` feature 时编译），包含房间 URL 解析、设备选择、进度条显示、键盘事件监听 |
| **Android JNI** | `src/android.rs` | JNI 函数导出，供 Android App 调用（仅 `target_os = "android"` 时编译），桥接所有引擎核心 API |
| **DLNA 控制** | `src/dlna_controller.rs` | SSDP 设备发现、`DlnaDevice` 结构体、`DlnaController` 封装，包含 AVTransport（SetAVTransportURI、Play、Pause、Seek、GetPositionInfo）和 RenderingControl（SetVolume、GetVolume）的 SOAP 调用，含 `controlURL` 兼容逻辑 |
| **Caster 抽象** | `src/cast/mod.rs` | `Caster` trait 定义统一投屏接口（`play_song`、`pause`、`resume`、`seek`、`get_progress`、`set_volume`、`get_volume`），以及 `Capabilities`、`Progress`、`SongRef` 等类型 |
| **DLNA 投屏** | `src/cast/dlna_caster.rs` | `Caster` 的 DLNA 实现，组合 `DlnaController` + `DlnaDevice` + 本地媒体服务器地址，将歌曲 URL 通过 SOAP 设置到设备并播放 |
| **Bilibili 投屏** | `src/cast/bilibili_caster.rs` | `Caster` 的 Bilibili 实现，包含 QR 扫码登录（`login_qr`）、设备列表获取（`list_devices`）、会话持久化（`save_session`/`load_session`）、通过 B 站 TV 投屏 API 发送命令（`send_cmd`） |
| **本地进度追踪** | `src/cast/progress.rs` | `LocalProgressTracker`，因为 Bilibili 模式无法获取设备端真实进度，此模块在本地模拟播放进度（`start`/`pause`/`resume`/`seek`/`get_progress`） |
| **媒体代理** | `src/media_server.rs` | actix-web HTTP 代理服务（proxy_handler），将 B 站视频直链转为 DLNA 设备可拉取的 HTTP 流，支持 Range 请求、eplus 鉴权参数补全、MP4 时长缓存 |
| **歌单管理** | `src/playlist_manager.rs` | 从 ktv-song-web 拉取/同步歌单，支持 WebSocket（`start_ws_update`）和 HTTP 轮询（`start_periodic_update`）两种模式，自动检测歌曲变更并触发投屏回调 |
| **B站解析** | `src/bilibili_parser.rs` | BV → aid 转换算法、获取视频分P信息（`get_page_info`）、获取视频直链（`get_bilibili_direct_link`） |
| **MP4 解析** | `src/mp4_util.rs` | 通过 Range 请求获取 MP4 文件头部（moov box），解析视频时长（`get_mp4_duration`），用于 DLNA 模式下展示正确进度条 |

---

## 当前歌曲 ID

`RustEngine.getCurrentSongId(): String` 对应 Rust 核心接口 `get_current_song_id_core()`，读取最近一次歌单同步的歌曲 ID。未连接或没有当前歌曲时返回空字符串，不发起网络请求。可与 `getCurrentSongTitle()` 一起轮询，用于区分同名歌曲；同一歌曲切换分 P 时 ID 保持不变。

## 当前歌曲分 P 接口

Rust 核心接口为 `get_current_bilibili_parts_core()` 和 `select_current_bilibili_part_core(song_id: &str, page_index: u32)`。对应的 `RustEngine` JNI 声明：

```kotlin
external fun getCurrentBilibiliParts(): String
external fun selectCurrentBilibiliPart(songId: String, pageIndex: Int): Int
```

两个方法均需在 IO 线程调用。查询返回 JSON 对象，包含 `songId`、`bvid`、`currentPageIndex` 和 `parts`；失败返回空字符串，原因记录在 Rust 日志中。每个分 P 包含 `pageIndex`、`page`、`cid`、`part`、`durationSecs`。索引从 0 开始，显示页码 `page` 从 1 开始，时长单位为秒，CID 使用 64 位整数。

选择时传入查询结果的 `songId` 和目标 `pageIndex`。返回 `1` 表示点歌台已接受修改，`0` 表示当前已是该分 P，`-1` 表示失败，详细原因见日志。`1` 不代表设备已经播放成功。

Rust 每次选择时读取房间当前歌曲，核对 ID，并取得最新标题、URL 和 hash，保留标题、通过 `/api/editCurrentSong` 保存目标 URL。调用方无需传递这些字段，Rust 也不维护查询缓存。提交前不重复读取歌单，由网页后端校验当前歌曲 ID 和 hash 后保存。修改请求不自动重试，提交超时后需刷新确认结果。

保存成功后由现有 WS／轮询同步触发目标分 P 从头播放，选择当前分 P 不写入、不重播。仅支持普通 BV 视频，兼容网页的一基 `p`、App 的零基 `page` 及内部 `BV号-page0` 格式；App 链接同时包含 `page` 和 `p` 时优先使用 `page`。

---

## 投屏模式详解

### DLNA 模式

#### 设备发现

程序支持两种发现方式：

1. **自动搜索（默认）**：通过 SSDP 多播 `239.255.255.250:1900`，发送 `M-SEARCH`（SearchTarget 为 AVTransport Service URN），监听设备返回的 `HTTP/1.1 200 OK`，解析 `LOCATION` 头获取设备描述 XML 地址
2. **手动输入 IP**：直接输入设备 IP（自动补全为 `http://{IP}:9958/bilibili/description.xml`）或完整的描述文件 URL（适用于 WiFi 不支持多播的场景）

```
设备发现方式：
  1. 自动搜索（SSDP 多播）
  2. 直接输入 IP
请选择 [1/2，默认1]：
```

#### 网络要求

- 运行机器与 DLNA 设备必须在同一局域网
- 需要允许 UDP 多播/广播（SSDP 发现依赖 `239.255.255.250:1900`）
- DLNA 设备需要能反向访问本机的媒体代理端口（默认 `8080`）
- 防火墙需放行入站 TCP 8080
- macOS 上若开启严格防火墙或第三方安全软件，常见现象是"能发现设备但无法播放"

#### 连接某些设备（如 ch**k）的特殊说明

某些品牌的包房设备需要先通过手机扫码激活投屏功能，DLNA 协议才能发现该设备：

1. 在包房设备上选择 **发现 → 手机投屏** 显示投屏二维码（**注意不是点歌二维码**）
2. **手机**：使用微信扫码 → 登录 → 同意用户协议 → 连接设备，成功后即可通过 DLNA 发现
3. **电脑**：用手机浏览器扫码获取链接，在**电脑版微信**中打开（注意使用微信内部浏览器，**不能用系统浏览器**）。MacOS 微信成功，Windows 版有时有 bug。如弹出允许访问本地网络设备，选择允许即可。

### Bilibili 模式

#### QR 扫码登录

选择 Bilibili 模式后，程序会通过 B 站的 `passport` 接口获取二维码 URL，在终端显示二维码（需要终端支持），使用 B 站 App 扫码即可登录。登录流程：

1. 调用 `x/passport-tv-login/qrcode/auth_code` 获取 `auth_code` 和 `qr_url`
2. `on_qr` 回调显示二维码
3. 轮询 `x/passport-tv-login/qrcode/poll` 等待扫码结果
4. 成功后保存 `BilibiliSession { access_token, mid }`

#### 会话持久化

登录成功后，Session 信息保存为 `bili_session.json` 文件。支持两种格式：

- **本工具格式**：`{"access_token": "...", "mid": 123}`
- **Python 格式兼容**：`{"data": {"token_info": {"access_token": "..."}, "mid": 123}}`

后续启动自动加载，无需重复扫码。

#### 设备配对与控制

通过 B 站 `x/tv/projection/devices` 接口获取在线设备列表，选择目标设备后通过 `x/tv/stream/cmd` 接口发送命令：

| 命令 | 值 | 说明 |
|------|-----|------|
| Play | 1 | 播放指定歌曲（需传入 aid/cid/type 等参数） |
| Pause | 5 | 暂停 |
| Resume | 6 | 继续 |
| Seek | 4 | 跳转到指定时间 |
| Stop | 7 | 停止 |

Bilibili 模式下，播放进度由本地 `LocalProgressTracker` 模拟，因为 B 站 TV 端不提供实时进度回传。

---

## 编译

### 环境要求

- **Rust**：稳定版（支持 `edition = "2024"` 的版本）
- 使用 `rustup` 安装

### Feature 说明

项目使用 Cargo features 管理编译配置：

| Feature | 依赖 | 说明 |
|---------|------|------|
| `default = ["cli"]` | `crossterm`, `indicatif`, `env_logger`, `qr2term` | 包含 CLI 交互界面 |
| `cli` | 同上 | 交互式命令行模式 |

Android 编译时需使用 `--no-default-features` 禁用 `cli` 依赖，避免编译非必要的桌面端 crate。

### 依赖说明

| 分类 | 依赖 | 用途 |
|------|------|------|
| Web 服务 | `actix-web`, `actix-files` | 本地媒体代理 HTTP 服务器 |
| 异步运行时 | `tokio` | 异步任务调度 |
| UPnP/DLNA | `rupnp`（patched） | 设备发现、SOAP 控制 |
| HTTP 客户端 | `reqwest`（rustls-tls） | 与点歌台 API、B站 API 通信 |
| WebSocket | `tokio-tungstenite`（rustls-tls） | 歌单实时同步 |
| 序列化 | `serde`, `serde_json` | JSON 解析 |
| 日志 | `log`, `env_logger`, `android_logger` | 调试输出 |
| MP4 解析 | `mp4` | 视频时长探测 |
| 其他 | `anyhow`, `url`, `md5`, `local-ip-address` | 通用工具 |

`rupnp` 使用了 GitHub 上的修补分支（`fix/control-endpoint-leading-slash`）以兼容某些 controlURL 不以 `/` 开头的设备。

### 桌面端

```bash
# 调试编译
cargo build

# 发布编译
cargo build --release
```

#### 运行

电脑端可直接运行：

```bash
cargo run
# 或指定日志等级
RUST_LOG=debug cargo run
```

### Android

```bash
# 安装 cargo-ndk
cargo install cargo-ndk

# 编译指定 ABI
cargo ndk -t arm64-v8a build --release --lib --no-default-features
```

支持的目标 ABI：`arm64-v8a`, `armeabi-v7a`, `x86_64`, `x86`

Android 编译产出 `libktv_casting_lib.so`，通过 JNI (`src/android.rs`) 为 Android App 提供 Native 引擎能力。

### 交叉编译

项目通过 GitHub Actions 使用 [cross-rs](https://github.com/cross-rs/cross) 进行交叉编译：

```bash
cross build --release --target x86_64-unknown-linux-musl
```

> 如果 crate 名称不是 `ktv_casting`（取决于代码里 `log::` 的 target），以实际为准；最稳妥还是用 `RUST_LOG=debug`。

## (重要!)连接DLNA设备

以ch**k为例子，需要先扫码后，包房的机器才能被DLNA协议发现
1. 在包房的平板上选择 发现-手机投屏，显示出投屏二维码(**注意不是点歌二维码**)
2. 连接设备(取决于你在哪里运行ktv-casting，是手机还是电脑)
- 手机：用手机微信扫码连接投屏，登录-同意用户协议-连接设备，看到成功的提示即可。此时打开b站或 BubbleUPNP等客户端测试，设备列表中会出现包房的机器名称。
- 电脑: 用手机浏览器扫码得到二维码对应的链接，在**电脑版微信**中打开链接(注意选择用微信内部浏览器，**不能使用系统浏览器**)，之后操作同手机端。MacOS的微信成功，但是Windows版有时会有bug。如果弹出允许访问本地网络的设备，选择允许即可。

## (重要!)辅助工具安装与抓包调试


### 手机端抓包

手机抓包的核心目标：

用一个“已知可用”的投屏 App（例如某些播放器）投同一个 DLNA 设备时，抓到它发的 SOAP 请求将其与本项目日志/抓包对比，找出差异（controlURL、SOAPAction、MetaData、协议字段等）

基本步骤：
1. 安装 [PCAPdroid](https://play.google.com/store/apps/details?id=com.emanuelef.remote_capture)
2. 启动捕获（会创建本地 VPN），选择导出到文件
3. 在bilibili等客户端连接，执行投屏操作
4. 结束捕获，导出 .pcap文件，在电脑端 Wireshark 打开

### 电脑端：Wireshark

安装[Wireshark](https://www.wireshark.org/download.html)

> MacOS安装时如果提示安装抓包权限组件（ChmodBPF），建议按提示完成，否则可能无法捕获某些接口流量。

### (重要!)常用 Display Filter和操作技巧

首先在ktv-casting的日志中获取设备ip, 记为`192.168.x.x`
常用抓包点：

1. SSDP 发现（UDP 1900，多播 239.255.255.250）
2. 设备描述与控制（HTTP：通常是设备的 80/1400/49152+ 等端口）
3. 你的本地媒体代理服务（HTTP：默认 8080）

### CI/CD

项目包含两套 GitHub Actions 工作流：

| 文件 | 触发条件 | 说明 |
|------|----------|------|
| `.github/workflows/build.yml` | push/PR 到 main | 基础跨平台编译（Android aarch64、Linux musl、Windows MSVC、macOS ARM64），打 tag 时自动发布 Release |
| `.github/workflows/build_with_android.yml` | tag v* 且 PR 到 android-app | 全量编译（Android 4 ABI + 桌面端） |

---
找到对应的包后，可以右键“Follow”→“HTTP Stream”查看完整请求/响应内容。点击“Back”返回抓包列表。

### 建议的抓包流程（定位“发现了但播不起来”）

1. 先确认 SSDP：能看到本机发出 `M-SEARCH`，也能看到设备返回 `HTTP/1.1 200 OK`
2. 点击设备返回里的 `LOCATION:`，确认能在浏览器访问到 `description.xml`
3. 观察程序调用 `SetAVTransportURI`/`Play` 时的 HTTP 请求：
   - URL 的 host/port/path 是否与 `description.xml` 的 `controlURL`/base URL 匹配
   - `SOAPAction` 是否正确
   - HTTP status 是否为 200
4. 观察 DLNA 设备是否来拉取媒体：是否访问了 `http://<你的IP>:8080/...`


### 其他辅助工具(可选)

[BubbleUPnP](https://bubblesoftapps.com/bubbleupnp/)（Android）：可以测试投屏到不同 DLNA 设备，确认设备是否支持 AVTransport 播放视频/音频。

- 投视频需要下载[VLC](https://www.videolan.org/vlc/index.zh.html)等播放器配合使用
- 在KTV使用DLNA工具可以参考[这篇帖子](https://www.xiaohongshu.com/discovery/item/68a01b0e000000001d01c489?source=webshare&xhsshare=pc_web&xsec_token=ABRib4kFPexc3iGS37nK9H-MDIYe91LBEGmU1hKU-oShk=&xsec_source=pc_share)

[扫码投屏的参考资料](https://dolphinstar.cn/)

## DLNA / UPnP 协议速览（结合本项目）

这里以"控制端（本程序）→ 渲染器（电视/盒子）"为主线。

### 1) SSDP 发现（UDP 1900）

- 控制端发送 `M-SEARCH` 到多播地址 `239.255.255.250:1900`
- 设备响应 `HTTP/1.1 200 OK`，包含 `LOCATION:`（设备描述 XML 的 URL）

Wireshark 中可看到：

- 请求：`M-SEARCH * HTTP/1.1`
- 关键头：
  - `ST:`（Search Target）
  - `MAN: "ssdp:discover"`
  - `MX:`

本项目使用 `rupnp::discover`，并以 `AVTransport` service URN 作为 SearchTarget（见 `src/dlna_controller.rs` 中 `AV_TRANSPORT` 常量）。

### 2) 设备描述（Device Description XML）

设备返回的 `LOCATION` 指向 `description.xml`，里面会列出：

- 设备类型：MediaRenderer/MediaServer
- serviceList：每个 service 的：
  - `serviceType`（例如 `urn:schemas-upnp-org:service:AVTransport:1`）
  - `controlURL`（SOAP 控制地址）
  - `eventSubURL`（事件订阅地址）
  - `SCPDURL`（服务描述）

**常见坑**：有些设备的 `controlURL` 不以 `/` 开头（例如 `_urn:...`）。本项目中有兼容逻辑：将控制 URL 强行拼成 `/_urn:schemas-upnp-org:service:AVTransport_control`（见 `avtransport_action_compat`）。

### 3) SOAP 控制（AVTransport）

投屏最核心的两个动作：

1. **`SetAVTransportURI`** — 设置要播放的媒体 URL
2. **`Play`** — 开始播放

请求形态：

- HTTP POST 到 `controlURL`
- Header：
  - `Content-Type: text/xml; charset="utf-8"`
  - `SOAPAction: "urn:schemas-upnp-org:service:AVTransport:1#SetAVTransportURI"`
- Body：SOAP Envelope + Action 参数

本项目额外做了 DIDL-Lite 元数据（`CurrentURIMetaData`）的构造与 XML escaping（见 `build_didl_lite_metadata`）。

### 4) 进度查询（GetPositionInfo）

渲染器不一定会主动上报进度，应用侧通常轮询：

- `GetPositionInfo` 返回 `RelTime`/`TrackDuration` 等

本项目的兼容实现会从 SOAP 返回 XML 中"尽力解析"常见 tag（`extract_xml_tag_value`），用来计算剩余/总时长。

### 5) RenderingControl（音量等，可选）

不少 DLNA 设备把音量、静音等放在 `RenderingControl` 服务。本项目支持 `SetVolume` / `GetVolume`。

---

## 调试与抓包

### 日志

DLNA 发现诊断使用 `DLNA_SSDP` 和 `DLNA_DISCOVERY` tag，在默认 INFO 等级可见：

- 自动搜索：直接使用 Tokio UDP 并行探测 AVTransport:1、MediaRenderer:1、ssdp:all。系统路由选择源 IPv4 后，三个未连接 UDP 套接字分别绑定此地址和独立端口，接收设备的单播响应；每个目标第 0/1 秒发送一次，MX=3、TTL=2，窗口5秒。
- `DLNA_SSDP` 记录真实的本地绑定地址、目的地址、发送字节、接收来源/长度、ST/USN/脱敏 LOCATION，区分无回包、响应被拒绝、重复、收发错误。每目标仅记录前50包详情但统计全部数据包。发送成功不证明设备已收到。按请求套接字归属对照结果，同时显示响应 ST。
- 三目标 LOCATION 合并去重后交给 rupnp 获取和解析设备描述，最多8个并发、单次3秒超时。`DLNA_DISCOVERY` 的结果项现在是设备描述获取结果数，不是 UDP 包数。保留 MediaRenderer 过滤与错误文本。
- 手动连接：区分地址解析失败、获取/解析描述失败和超时，并记录端点、设备类型和服务。
- 日志中的描述端点去除用户名、密码、查询参数和 fragment；设备名称与局域网地址仍用于诊断。

JNI 继续返回原有设备数组，空数组可能表示无结果或发现失败；应结合上述终态日志判断，不将空结果直接解释为设备离线。设备筛选和播放行为保持不变。搜索目标扩大为三种，各自的统计独立，设备描述网络请求在5秒探测完成后进行。

```bash
# 完整调试日志
RUST_LOG=debug cargo run

# 仅查看本项目的日志
RUST_LOG=info,ktv_casting_lib=debug cargo run
```

### Wireshark 抓包

推荐使用 Wireshark 抓包比对正常投屏与本项目的 SOAP 请求差异。

#### 常用 Display Filter

| 场景 | Wireshark Display Filter |
|------|-------------------------|
| SSDP 发现 | `ip.addr == 192.168.x.x && udp.port == 1900` |
| DLNA 控制 (SOAP) | `ip.addr == 192.168.x.x && http` |
| AVTransport 控制 | `http contains "AVTransport"` |
| SetAVTransportURI（请求体长） | `ip.addr == 192.168.x.x && http && frame.len >= 1000` |
| Play 请求 | `ip.addr == 192.168.x.x && http && http.request.method == "POST" && http contains "Play"` |
| 本地媒体代理 | `tcp.port == 8080` |

在对应包上右键 → **Follow → HTTP Stream** 可查看完整请求/响应内容。

#### 建议的抓包流程（定位"发现了但播不起来"）

1. 确认 SSDP：能看到本机发出 `M-SEARCH`，设备返回 `HTTP/1.1 200 OK`
2. 点击设备返回里的 `LOCATION:`，确认能在浏览器访问到 `description.xml`
3. 观察程序调用 `SetAVTransportURI`/`Play` 时的 HTTP 请求：
   - URL 的 host/port/path 是否与 `description.xml` 的 `controlURL`/base URL 匹配
   - `SOAPAction` 是否正确
   - HTTP status 是否为 200
4. 观察 DLNA 设备是否来拉取媒体：是否访问了 `http://<你的IP>:8080/...`

### 手机抓包

目标：用一个"已知可用"的投屏 App 投同一个 DLNA 设备时，抓到它发的 SOAP 请求，与本项目日志/抓包对比，找出差异（controlURL、SOAPAction、MetaData、协议字段等）。

步骤：

1. 安装 [PCAPdroid](https://play.google.com/store/apps/details?id=com.emanuelef.remote_capture)
2. 启动捕获（会创建本地 VPN）
3. 在 Bilibili 等客户端连接，执行投屏操作
4. 导出 .pcap 文件，在电脑端 Wireshark 打开

### 辅助工具

| 工具 | 链接 | 用途 |
|------|------|------|
| Wireshark | https://www.wireshark.org/download.html | 网络抓包分析 |
| PCAPdroid | https://play.google.com/store/apps/details?id=com.emanuelef.remote_capture | Android 手机抓包 |
| BubbleUPnP | https://bubblesoftapps.com/bubbleupnp/ | 测试 DLNA 设备兼容性，需配合 [VLC](https://www.videolan.org/vlc/) 播放器 |
| 小红书参考帖 | https://www.xiaohongshu.com/discovery/item/68a01b0e000000001d01c489 | KTV 使用 DLNA 工具参考 |

---

## 常见问题（FAQ）

### 能发现设备，但 SetAVTransportURI/Play 没效果

优先按顺序排查：

1. 设备是否真的支持 `AVTransport:1`（某些只支持投图片/镜像）
2. `controlURL` 拼接是否正确（特别是缺 `/` 的设备）
3. 渲染器是否能访问你的 `http://<IP>:8080/...`（防火墙/跨网段/NAT/手机热点都常见）
4. URL/MetaData 是否被设备拒绝：
   - MIME 类型不匹配
   - `protocolInfo` 太严格
   - `CurrentURIMetaData` 缺字段或转义有误

### 设备拉取 8080 失败

- 看 Wireshark：是否有来自设备的 TCP SYN 到 8080
- 若无：设备根本访问不到你机器（网络隔离/访客 Wi‑Fi/跨 VLAN）
- 若有但被 reset：本机防火墙/安全软件

### 抓包里看到 HTTP 204/401/500

- **204**：少数设备会返回非 200 但实际上成功（项目里对部分 2xx error code 有"视为成功"的处理）
- **401**：需要认证（少见）或是走错了 controlURL
- **500**：多数是 SOAP 参数不匹配/MetaData 格式不被接受

### Bilibili 模式登录失败

- 确认二维码未过期（有效期约 2 分钟）
- 确认 B 站 App 版本支持 TV 登录
- 检查网络是否能访问 `passport.snm0516.aisee.tv` 和 `api.bilibili.com`

---

## 已知问题与排查记录

### [2026-08-03] DLNA 模式暂停失效 + App 闪退（复现中）

**现象**
- CLI（`cargo run --release`，DLNA 模式选小电视，房间 111）：一开始暂停/恢复正常，播放一段时间后按 `p` 无响应
- Android App 同样问题，且主线程卡死在 native（ANR）后进程被杀（闪退）
- 设备：B站小电视 `http://<IP>:9958/bilibili/description.xml`；本机环境日志见 `debuglog.txt`

**根因（两个叠加缺陷）**

1. **SOAP 调用无超时 → 闪退的直接原因**
   - rupnp `Service::action()` 和 compat 路径的 reqwest 请求都没有 `.timeout()`。
   - 小电视不响应时 `await` **永久挂起**。
   - App JNI 层（`src/android.rs`）在持有 `ENGINE_STATE` 读锁的情况下 `ctx.rt.block_on(...)` 同步等待 —— 主线程永久卡死 → ANR → 进程被杀（闪退）。
   - `debuglog.txt` 里 Signal Catcher 收到 SIGQUIT 后 `libbacktrace: Timed out waiting for signal handler`，确认主线程卡死在 native 无法响应。

2. **每次 SOAP 调用新建 TCP 连接 → 播放一段时间后设备被打崩**
   - rupnp `action()` 每次内部 `Client::builder(...).build_http()`；进度轮询每秒一次 GetPositionInfo = **每秒向设备开一条新 TCP 连接**。
   - 长时间运行耗尽设备 UPnP 服务连接容量，设备停止响应 → 触发缺陷 1。

**完整因果链（从 UI 到闪退）**

```
Java UI 线程
  → JNI: RustEngine_togglePause()              // src/android.rs:281
    → ENGINE_STATE.read()                       // 持有读锁
      → ctx.rt.block_on(toggle_pause_core())    // 同步阻塞，等 async 完成
        → caster.pause()                        // Caster trait
          → DlnaCaster::pause()                 // src/cast/dlna_caster.rs:45
            → controller.pause(&device)         // src/dlna_controller.rs:597
              → avtransport_action_compat(...)  // 发 SOAP Pause 请求
                → service.action(...).await     // ❌ 没有 .timeout()
                  或
                → reqwest::Client::post(...).send().await  // ❌ 也没有 .timeout()
```

`service.action().await` 内部 `Client::builder().build_http().request(...)` 每次新建 TCP 连接且无超时。
设备不响应 → `await` 永不返回 → `block_on` 永不返回 → 读锁永不释放 → **Java 主线程卡死在 native**。

Android 系统 5 秒后检测到主线程无响应 → ANR → 发送 SIGQUIT 采集 backtrace
→ 但线程卡在 `block_on` 里连信号都处理不了 → `libbacktrace: Timed out waiting for signal handler`
→ 进程被杀（闪退）。

CLI 同理：键盘事件处理器 `rt.block_on(...)` 卡死 → 终端假死，按 `p` 无反应。

**修复（`git diff` 可见）**

| 修复 | 文件 | 说明 |
|------|------|------|
| SOAP 全链路超时 | `src/dlna_controller.rs` | `timeout_soap()` 包裹 native action + RenderingControl 调用（3s），兼容模式单请求 1.5s；设备不响应时返回错误而非永久挂起 |
| 连接复用 | `src/dlna_controller.rs` | 新增全局 `soap_client()`（连接池 + 超时），`avtransport_action_compat` 首选走共享 client；rupnp native action 降为兜底。消除每秒新建连接的根源 |
| CLI 错误可见 | `src/main.rs` | 暂停失败时 `warn!` 打日志（原静默吞掉），锁污染时恢复而非 panic |
| 防御性修复 | `src/lib.rs` / `src/playlist_manager.rs` | 媒体代理 `connect_timeout(10s)`；WS 重连前 `sleep(3s)` 退避 |

**验证（真实小电视 + 房间 111，13 分钟长跑，3 首歌）**：暂停 49 次全成功，0 次 SOAP 超时，连接复用 100%。

---

## 贡献与开发建议

- 优先把"可复现问题"的抓包（pcap）和日志（`RUST_LOG=debug`）一起提交
- 不同电视/盒子兼容性差异很大：新增兼容逻辑时建议以抓包为基准
- 对控制 URL、SOAPAction、MetaData 可考虑做可配置化
- 提交前确保现有测试通过：`cargo test`
