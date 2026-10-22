use std::time::Duration;

// ── 超时常量 ──────────────────────────────────────────────
// 所有可能阻塞 App 主线程（JNI block_on）或 CLI 键盘事件的
// 同步等待都必须在此阈值内返回，否则会触发 Android ANR（5s）。
pub const ANR_DEADLINE: Duration = Duration::from_secs(5);

/// DLNA SOAP 请求超时。局域网内正常响应 <100ms，3s 留有充足余量。
pub const SOAP_TIMEOUT: Duration = Duration::from_secs(3);
const _: () = assert!(SOAP_TIMEOUT.as_secs() < ANR_DEADLINE.as_secs(),
    "SOAP_TIMEOUT 必须小于 ANR_DEADLINE (5s)，否则设备不响应时无法在 ANR 前返回");

/// B站 API / 点歌台 HTTP 请求超时。
pub const API_TIMEOUT: Duration = Duration::from_secs(3);
const _: () = assert!(API_TIMEOUT.as_secs() < ANR_DEADLINE.as_secs(),
    "API_TIMEOUT 必须小于 ANR_DEADLINE (5s)，否则 B 站 API 不响应时无法在 ANR 前返回");

/// 媒体代理上游连接超时。此路径不在 block_on 链路上，可设得更宽松。
pub const PROXY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
