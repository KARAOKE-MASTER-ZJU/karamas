use std::time::Duration;

/// DLNA SOAP 请求超时。
pub const SOAP_TIMEOUT: Duration = Duration::from_secs(3);

/// B站 API / 点歌台 HTTP 请求超时。
pub const API_TIMEOUT: Duration = Duration::from_secs(3);

/// 媒体代理上游连接超时。
pub const PROXY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
