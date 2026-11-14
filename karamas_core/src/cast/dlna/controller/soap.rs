//! SOAP 请求、响应解析、连接复用和 AVTransport 端点兼容回退。
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use rupnp::http::Uri;
use std::{collections::HashMap, future::Future, sync::OnceLock, time::Duration};
use tokio::time::timeout;

/// 兼容模式的单次 SOAP 请求超时。
const SOAP_TIMEOUT_COMPAT: Duration = Duration::from_millis(1500);

/// 全局复用的 SOAP HTTP 客户端：连接池复用 + 统一超时。
/// 修复前：每次 action 都新建 reqwest Client（新 TCP 连接），配合 1s 一次的
/// GetPositionInfo 轮询，等于每秒向设备开一条新连接，长时间运行会打崩设备
/// 的 UPnP 服务（导致其停止响应）。
fn soap_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(crate::config::SOAP_TIMEOUT)
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .expect("创建 SOAP HTTP 客户端失败")
    })
}

/// 给 rupnp action future 加超时。设备不响应时返回超时错误，而不是永久等待。
pub(super) async fn timeout_soap<T>(
    fut: impl Future<Output = Result<T, rupnp::Error>>,
) -> Result<T, rupnp::Error> {
    match timeout(crate::config::SOAP_TIMEOUT, fut).await {
        Ok(r) => r,
        Err(_) => Err(rupnp::Error::ParseError("SOAP 调用超时")),
    }
}

fn build_soap_envelope(action: &str, args_xml: &str) -> String {
    // Keep the shape consistent with what most renderers accept (and close to your B站抓包).
    // Note: `rupnp` will build its own envelope too, but we log a best-effort equivalent
    // so you can diff with a packet capture.
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<s:Envelope s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/" xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
        <u:{action} xmlns:u="urn:schemas-upnp-org:service:AVTransport:1">{args}</u:{action}>
  </s:Body>
</s:Envelope>"#,
        action = action,
        args = args_xml
    )
}

pub(super) fn log_upnp_action(
    service: &rupnp::Service,
    base_url: &Uri,
    action: &str,
    args_xml: &str,
) {
    // `service.action()` internally ends up using control_url(base_url).
    // We can't call the private control_url() here, so we log the base_url and also log
    // the serviceId/type to help confirm we matched the expected service.
    //
    // If you still see 204, compare this log with your抓包，重点确认：Host/port/path。
    let soap_action_header = format!("\"urn:schemas-upnp-org:service:AVTransport:1#{}\"", action);

    // Logged body is a best-effort “wire-like” payload for diffing.
    let envelope = build_soap_envelope(action, args_xml);

    log::trace!(
        "UPnP Action -> base_url={} service_id={} service_type={} SOAPAction={}",
        base_url,
        service.service_id(),
        service.service_type(),
        soap_action_header
    );
    log::trace!("UPnP Action body (approx) => {}", envelope);
}

/// 按元素本地名读取对应 action 响应的直接子字段。
fn parse_soap_response_fields(
    text: &str,
    action: &str,
) -> Result<HashMap<String, String>, rupnp::Error> {
    let document = roxmltree::Document::parse(text).map_err(rupnp::Error::invalid_response)?;
    let body = document
        .root_element()
        .children()
        .find(|node| node.has_tag_name(("http://schemas.xmlsoap.org/soap/envelope/", "Body")))
        .ok_or(rupnp::Error::ParseError("SOAP 响应缺少 Body"))?;
    let response_name = format!("{}Response", action);
    let response = body
        .children()
        .find(|node| node.has_tag_name(response_name.as_str()))
        .ok_or(rupnp::Error::ParseError("SOAP 响应缺少对应的 action 响应"))?;

    Ok(response
        .children()
        .filter(|node| node.is_element())
        .map(|node| {
            (
                node.tag_name().name().to_owned(),
                node.text().unwrap_or_default().trim().to_owned(),
            )
        })
        .collect())
}

fn soap_http_failure(status: reqwest::StatusCode, text: &str) -> String {
    let mut summary = format!("HTTP={}", status.as_u16());
    if let Ok(document) = roxmltree::Document::parse(text) {
        if let Some(fault) = document
            .descendants()
            .find(|node| node.has_tag_name("Fault"))
        {
            let field = |name| {
                fault
                    .descendants()
                    .find(|node| node.has_tag_name(name))
                    .and_then(|node| node.text())
                    .map(str::trim)
            };
            let code = field("errorCode").unwrap_or("未知");
            let description = field("errorDescription")
                .or_else(|| field("faultstring"))
                .unwrap_or("未提供说明");
            summary.push_str(&format!("，SOAP错误={} {}", code, description));
        }
    }
    if text.trim().is_empty() {
        summary.push_str("，响应体为空");
    }
    summary
}

fn soap_request_failure(
    action: &str,
    url: &str,
    stage: &str,
    detail: impl std::fmt::Display,
) -> rupnp::Error {
    let message = format!(
        "[DLNA] {}失败：步骤={}，端点={}，{}",
        action, stage, url, detail
    );
    log::warn!("{}", message);
    rupnp::Error::invalid_response(std::io::Error::other(message))
}

/// 用全局共享 client（连接池复用 + 统一超时）发送一次 SOAP 请求到指定 URL。
/// 这是主要 SOAP 通道：避免 rupnp 原生 action 每次新建 TCP 连接、每秒打一次设备，
/// 长时间运行把设备 UPnP 服务打崩。
async fn send_soap_shared(
    final_url: &str,
    action: &str,
    args_xml: &str,
    timeout: Duration,
    retry_timeout: bool,
) -> Result<HashMap<String, String>, rupnp::Error> {
    let soap_action_header = format!("\"urn:schemas-upnp-org:service:AVTransport:1#{}\"", action);
    let body = build_soap_envelope(action, args_xml);

    log::trace!(
        "UPnP Action (shared) -> url={} SOAPAction={}",
        final_url,
        soap_action_header
    );

    let mut headers = HeaderMap::new();
    headers.insert(
        "SOAPAction",
        HeaderValue::from_str(&soap_action_header)
            .map_err(|_| rupnp::Error::ParseError("SOAPAction header非法"))?,
    );
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/xml; charset=\"utf-8\""),
    );

    let send = || async {
        soap_client()
            .post(final_url)
            .timeout(timeout)
            .headers(headers.clone())
            .body(body.clone())
            .send()
            .await
    };
    let resp = if retry_timeout {
        let context = format!("[DLNA] 发送 {}：端点={}", action, final_url);
        crate::cast::request::retry_cast_request(&context, send).await
    } else {
        send().await
    }
    .map_err(|error| {
        let detail = if error.is_timeout() {
            "等待响应头超时".to_owned()
        } else {
            error.to_string()
        };
        soap_request_failure(action, final_url, "发送SOAP请求", detail)
    })?;
    let status = resp.status();
    let text = resp.text().await.map_err(|error| {
        soap_request_failure(
            action,
            final_url,
            "读取SOAP响应",
            format!("HTTP={}，错误={}", status.as_u16(), error),
        )
    })?;

    if status.as_u16() == 200 {
        let fields = parse_soap_response_fields(&text, action).map_err(|error| {
            soap_request_failure(
                action,
                final_url,
                "解析SOAP响应",
                format!("HTTP=200，错误={}", error),
            )
        })?;
        // 正常 SOAP 请求与响应明细仅在 trace 级别输出。
        log::trace!(
            "UPnP Action (shared) succeeded: action={}, url={}",
            action,
            final_url
        );
        log::trace!("UPnP Action (shared) status=200 body={}", text);
        Ok(fields)
    } else {
        let message = format!(
            "[DLNA] {}失败：步骤=设备响应，端点={}，{}",
            action,
            final_url,
            soap_http_failure(status, &text)
        );
        log::warn!("{}，响应体={}", message, text);
        Err(rupnp::Error::invalid_response(std::io::Error::other(
            message,
        )))
    }
}

/// Some renderers publish a `controlURL` like `_urn:schemas-upnp-org:service:AVTransport_control`
/// (missing the leading `/`). In practice the working endpoint is often `/_urn:...`.
///
/// `rupnp`'s internal URL replacement may produce the wrong path for such devices.
/// To make behavior explicit (and loggable), we send the SOAP request ourselves to:
/// `{scheme}://{host}:{port}/{control_path}`。
///
/// 发送顺序（都带超时，设备不响应时优雅降级而不是无限等待）：
///   1. 首选：从 Service Debug 提取 control endpoint，用共享 client 发送（连接池复用）
///   2. 兜底：rupnp 原生 action（Debug 解析失败的设备；每次新建连接，尽量少走）
///   3. 最后：常见 control 路径遍历（共享 client，短超时）
pub(super) async fn avtransport_action_compat(
    service: &rupnp::Service,
    base_url: &Uri,
    action: &str,
    args_xml: &str,
    retry_timeout: bool,
) -> Result<HashMap<String, String>, rupnp::Error> {
    let host = base_url
        .host()
        .ok_or(rupnp::Error::ParseError("base_url缺少host"))?
        .to_string();
    let scheme = base_url
        .scheme_str()
        .ok_or(rupnp::Error::ParseError("base_url缺少scheme"))?;
    let port = base_url
        .port_u16()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });

    let service_debug = format!("{:?}", service);
    let mut first_failure = None;
    let mut primary_endpoint = None;

    // 1) 首选：Debug 提取 control endpoint，走共享 client（连接复用，避免每秒新建连接打崩设备）
    if let Some(path) = extract_control_endpoint_from_debug(&service_debug) {
        let p = normalize_control_path(&path);
        let final_url = if p.starts_with("http://") || p.starts_with("https://") {
            p
        } else {
            format!("{}://{}:{}{}", scheme, host, port, p)
        };
        primary_endpoint = Some(final_url.clone());
        match send_soap_shared(
            &final_url,
            action,
            args_xml,
            crate::config::SOAP_TIMEOUT,
            retry_timeout,
        )
        .await
        {
            Ok(response) => return Ok(response),
            Err(error) => {
                first_failure = Some(error.to_string());
                log::debug!(
                    "[DLNA] {}首选端点失败，回退至原生接口：端点={}",
                    action,
                    final_url
                );
            }
        }
    }

    // 2) 兜底：rupnp 原生 action（每次新建连接，仅用于 Debug 解析失败/路径不匹配的设备）
    match timeout_soap(service.action(base_url, action, args_xml)).await {
        Ok(response) => {
            log::trace!("UPnP Action (native) succeeded");
            log::trace!("UPnP Action (native) response: {:?}", response);
            return Ok(response);
        }
        Err(e) => {
            let endpoint = primary_endpoint.unwrap_or_else(|| base_url.to_string());
            let message = format!(
                "[DLNA] {}原生接口失败：端点={}，错误={}，继续尝试候选路径",
                action, endpoint, e
            );
            log::warn!("{}", message);
            first_failure.get_or_insert(message);
        }
    }

    // 3) 最后兜底：常见 control 路径遍历（共享 client，短超时）
    log::trace!("Service Debug info: {}", service_debug);
    let mut possible_paths: Vec<String> = Vec::new();

    // 尝试从 debug 中解析出真实的控制路径（常见于 Windows UPnP Host）
    if let Some(start) = service_debug.find("/upnphost/udhisapi.dll?control=")
        && let Some(end) = service_debug[start..].find(", event_sub_endpoint")
    {
        let real_path = &service_debug[start..start + end];
        possible_paths.push(normalize_control_path(real_path));
    }

    // 通用回退路径
    possible_paths.extend(
        [
            "_urn:schemas-upnp-org:service:AVTransport_control",
            "AVTransport/control",
            "upnp/control/AVTransport",
            "control/AVTransport",
        ]
        .into_iter()
        .map(normalize_control_path), // 规范化路径,增加/
    );

    for path in possible_paths {
        let final_url = if path.starts_with("http://") || path.starts_with("https://") {
            path
        } else {
            format!("{}://{}:{}{}", scheme, host, port, path)
        };
        match send_soap_shared(
            &final_url,
            action,
            args_xml,
            SOAP_TIMEOUT_COMPAT,
            retry_timeout,
        )
        .await
        {
            Ok(response) => return Ok(response),
            Err(e) => {
                first_failure.get_or_insert_with(|| e.to_string());
                log::debug!(
                    "[DLNA] {}回退候选失败：端点={}，错误={}",
                    action,
                    final_url,
                    e
                );
            }
        }
    }

    // 所有尝试都失败
    let message = format!(
        "[DLNA] {}失败：所有控制端点均失败；首个失败原因={}",
        action,
        first_failure.as_deref().unwrap_or("没有可用控制端点")
    );
    log::warn!("{}", message);
    Err(rupnp::Error::invalid_response(std::io::Error::other(
        message,
    )))
}

fn normalize_control_path(path: &str) -> String {
    let p = path.trim();
    if p.starts_with("http://") || p.starts_with("https://") {
        return p.to_string();
    }
    if p.starts_with('/') {
        p.to_string()
    } else {
        format!("/{}", p)
    }
}

fn extract_control_endpoint_from_debug(service_debug: &str) -> Option<String> {
    if let Some(start) = service_debug.find("control_endpoint: ") {
        let start = start + "control_endpoint: ".len();
        if let Some(end) = service_debug[start..].find(", event_sub_endpoint") {
            let path = service_debug[start..start + end].trim();
            Some(path.to_string())
        } else {
            None
        }
    } else {
        None
    }
}

#[cfg(test)]
#[path = "tests/diagnostics.rs"]
mod diagnostics;
