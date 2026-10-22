use super::{
    client::{API_HOST, APPKEY, bili_client},
    session::BilibiliSession,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BilibiliDevice {
    pub name: String,
    pub brand: String,
    pub model: String,
    pub buvid: String,
}

impl std::fmt::Display for BilibiliDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}-{}) [{}]", self.name, self.brand, self.model, self.buvid)
    }
}

fn is_auth_error(code: i64, message: &str) -> bool {
    code == -101
        || message.contains("未登录")
        || message.to_ascii_lowercase().contains("access_key")
        || message.to_ascii_lowercase().contains("token")
}

fn parse_devices_response(json: Value) -> Result<Vec<BilibiliDevice>, String> {
    if let Some(code) = json["code"].as_i64() {
        if code != 0 {
            let message = json["message"]
                .as_str()
                .or_else(|| json["msg"].as_str())
                .unwrap_or("unknown error");
            let prefix = if is_auth_error(code, message) {
                "auth expired"
            } else {
                "api error"
            };
            return Err(format!("{prefix}: code={code}, message={message}"));
        }
    }

    let Some(arr) = json["data"]["devices"].as_array() else {
        return Err("no devices array".into());
    };

    Ok(arr
        .iter()
        .filter_map(|d| {
            Some(BilibiliDevice {
                name: d["name"].as_str()?.into(),
                brand: d["brand"].as_str()?.into(),
                model: d["model"].as_str()?.into(),
                buvid: d["buvid"].as_str()?.into(),
            })
        })
        .collect())
}

pub async fn list_devices(session: &BilibiliSession) -> Result<Vec<BilibiliDevice>, String> {
    let endpoint = format!("{}/x/tv/projection/devices", API_HOST);
    let started = Instant::now();
    let response = bili_client()
        .get(&endpoint)
        .query(&[
            ("access_key", session.access_token.as_str()),
            ("appkey", APPKEY),
        ])
        .send()
        .await
        .map_err(|error| devices_request_failure("发送请求", &endpoint, started, None, error))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| devices_request_failure("读取响应", &endpoint, started, Some(status), error))?;
    let json: Value = serde_json::from_slice(&bytes).map_err(|error| format!(
        "[Bilibili] 获取设备列表失败：步骤=解析JSON响应，请求=GET {endpoint}，耗时={}ms，超时设置={}ms，HTTP={}，错误链={error}",
        started.elapsed().as_millis(), crate::config::API_TIMEOUT.as_millis(), status.as_u16(),
    ))?;

    parse_devices_response(json)
}

fn devices_request_failure(
    stage: &str,
    endpoint: &str,
    started: Instant,
    status: Option<reqwest::StatusCode>,
    error: reqwest::Error,
) -> String {
    // reqwest's URL contains access_key; remove it before formatting the error chain.
    let error = error.without_url();
    let mut causes = vec![error.to_string()];
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        causes.push(cause.to_string());
        source = cause.source();
    }
    let http = status.map(|status| format!("，HTTP={}", status.as_u16())).unwrap_or_default();
    format!(
        "[Bilibili] 获取设备列表失败：步骤={stage}，请求=GET {endpoint}，耗时={}ms，超时设置={}ms{http}，is_timeout={}，is_connect={}，is_request={}，is_body={}，is_decode={}，错误链={}",
        started.elapsed().as_millis(),
        crate::config::API_TIMEOUT.as_millis(),
        error.is_timeout(),
        error.is_connect(),
        error.is_request(),
        error.is_body(),
        error.is_decode(),
        causes.join(" -> "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_devices_response_surfaces_auth_expiry() {
        let err = parse_devices_response(json!({
            "code": -101,
            "message": "账号未登录"
        }))
        .unwrap_err();

        assert!(err.starts_with("auth expired:"));
        assert!(err.contains("账号未登录"));
    }
}
