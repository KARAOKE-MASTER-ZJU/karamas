//! 解析设备及服务描述，发送 SOAP 动作并记录返回字段。
use super::http::{TestResponse, client, exchange, http_url};
use crate::CoreError;
use roxmltree::{Document, Node};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    pub service_type: String,
    pub service_id: String,
    pub scpd_url: String,
    pub control_url: String,
    pub event_sub_url: String,
}

#[derive(Serialize)]
pub struct DeviceInfo {
    pub response: TestResponse,
    pub services: Vec<Service>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateVariable {
    pub name: String,
    pub data_type: String,
    pub default_value: String,
    pub allowed_values: Vec<String>,
    pub minimum: String,
    pub maximum: String,
    pub step: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Argument {
    pub name: String,
    pub direction: String,
    pub related_state_variable: String,
}

#[derive(Serialize)]
pub struct Action {
    pub name: String,
    pub arguments: Vec<Argument>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInfo {
    pub response: TestResponse,
    pub actions: Vec<Action>,
    pub state_variables: Vec<StateVariable>,
}

fn text(node: Node<'_, '_>, name: &str) -> String {
    node.children()
        .find(|child| child.has_tag_name(name))
        .and_then(|child| child.text())
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn fields(node: Node<'_, '_>) -> BTreeMap<String, String> {
    node.children()
        .filter(|child| child.is_element() && !child.children().any(|n| n.is_element()))
        .map(|child| {
            (
                child.tag_name().name().to_owned(),
                child.text().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn parse_device(response: &mut TestResponse) -> Result<Vec<Service>, CoreError> {
    let document = Document::parse(&response.raw)?;
    let root = document.root_element();
    let device = root
        .children()
        .find(|node| node.has_tag_name("device"))
        .ok_or("响应缺少 device")?;
    response.fields = fields(device);
    let base_text = text(root, "URLBase");
    let base = http_url(if base_text.is_empty() {
        &response.url
    } else {
        &base_text
    })?;
    let mut services = Vec::new();
    for node in device
        .descendants()
        .filter(|node| node.has_tag_name("service"))
    {
        let endpoint = |name| -> Result<String, CoreError> {
            let value = text(node, name);
            if value.is_empty() {
                return Ok(String::new());
            }
            Ok(http_url(base.join(&value)?.as_str())?.to_string())
        };
        services.push(Service {
            service_type: text(node, "serviceType"),
            service_id: text(node, "serviceId"),
            scpd_url: endpoint("SCPDURL")?,
            control_url: endpoint("controlURL")?,
            event_sub_url: endpoint("eventSubURL")?,
        });
    }
    Ok(services)
}

pub async fn device_info(location: &str) -> Result<DeviceInfo, CoreError> {
    let url = http_url(location)?;
    let mut response = exchange(client(true).get(url), location, String::new(), true).await;
    let services = match parse_device(&mut response) {
        Ok(services) => services,
        Err(error) => {
            response.error.get_or_insert(error.to_string());
            Vec::new()
        }
    };
    Ok(DeviceInfo { response, services })
}

async fn service(location: &str, service_id: &str) -> Result<Service, CoreError> {
    let info = device_info(location).await?;
    info.services
        .into_iter()
        .find(|item| item.service_id == service_id)
        .ok_or_else(|| {
            info.response
                .error
                .unwrap_or_else(|| format!("设备未返回服务：{service_id}"))
                .into()
        })
}

fn parse_service(raw: &str) -> Result<(Vec<Action>, Vec<StateVariable>), CoreError> {
    let document = Document::parse(raw)?;
    let root = document.root_element();
    if !root.has_tag_name("scpd") {
        return Err("响应缺少 scpd".into());
    }
    let actions = root
        .descendants()
        .filter(|node| node.has_tag_name("action"))
        .map(|node| Action {
            name: text(node, "name"),
            arguments: node
                .descendants()
                .filter(|node| node.has_tag_name("argument"))
                .map(|arg| Argument {
                    name: text(arg, "name"),
                    direction: text(arg, "direction"),
                    related_state_variable: text(arg, "relatedStateVariable"),
                })
                .collect(),
        })
        .collect();
    let variables = root
        .descendants()
        .filter(|node| node.has_tag_name("stateVariable"))
        .map(|node| StateVariable {
            name: text(node, "name"),
            data_type: text(node, "dataType"),
            default_value: text(node, "defaultValue"),
            allowed_values: node
                .descendants()
                .filter(|node| node.has_tag_name("allowedValue"))
                .map(|node| node.text().unwrap_or_default().to_owned())
                .collect(),
            minimum: node
                .descendants()
                .find(|node| node.has_tag_name("minimum"))
                .and_then(|node| node.text())
                .unwrap_or_default()
                .to_owned(),
            maximum: node
                .descendants()
                .find(|node| node.has_tag_name("maximum"))
                .and_then(|node| node.text())
                .unwrap_or_default()
                .to_owned(),
            step: node
                .descendants()
                .find(|node| node.has_tag_name("step"))
                .and_then(|node| node.text())
                .unwrap_or_default()
                .to_owned(),
        })
        .collect();
    Ok((actions, variables))
}

pub async fn service_info(location: &str, service_id: &str) -> Result<ServiceInfo, CoreError> {
    let service = service(location, service_id).await?;
    let url = http_url(&service.scpd_url)?;
    let mut response = exchange(
        client(true).get(url),
        &service.scpd_url,
        String::new(),
        true,
    )
    .await;
    let (actions, state_variables) = match parse_service(&response.raw) {
        Ok(info) => info,
        Err(error) => {
            response.error.get_or_insert(error.to_string());
            (Vec::new(), Vec::new())
        }
    };
    Ok(ServiceInfo {
        response,
        actions,
        state_variables,
    })
}

pub(super) async fn soap(
    service: &Service,
    action: &str,
    args: &[(String, String)],
) -> Result<TestResponse, CoreError> {
    let url = http_url(&service.control_url)?;
    let args = args
        .iter()
        .map(|(name, value)| format!("<{name}>{}</{name}>", xml_escape(value)))
        .collect::<String>();
    let payload = format!(
        r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><s:Body><u:{action} xmlns:u="{}">{args}</u:{action}></s:Body></s:Envelope>"#,
        xml_escape(&service.service_type)
    );
    let request = client(true)
        .post(url)
        .header("Content-Type", "text/xml; charset=utf-8")
        .header(
            "SOAPAction",
            format!("\"{}#{action}\"", service.service_type),
        )
        .body(payload.clone());
    let mut response = exchange(request, &service.control_url, payload, true).await;
    let parsed = (|| -> Result<BTreeMap<String, String>, CoreError> {
        let document = Document::parse(&response.raw)?;
        let body = document
            .descendants()
            .find(|node| node.has_tag_name("Body"))
            .ok_or("响应缺少 Body")?;
        let reply = body.first_element_child().ok_or("Body 为空")?;
        if reply.has_tag_name("Fault") {
            return Ok(reply
                .descendants()
                .filter(|node| node.is_element() && !node.children().any(|n| n.is_element()))
                .map(|node| {
                    (
                        node.tag_name().name().to_owned(),
                        node.text().unwrap_or_default().to_owned(),
                    )
                })
                .collect());
        }
        if reply.tag_name().name() != format!("{action}Response") {
            return Err("响应动作不匹配".into());
        }
        Ok(fields(reply))
    })();
    match parsed {
        Ok(fields) => response.fields = fields,
        Err(error) => {
            response.error.get_or_insert(error.to_string());
        }
    }
    Ok(response)
}

pub async fn query(
    location: &str,
    service_id: &str,
    action: &str,
    args: BTreeMap<String, String>,
) -> Result<TestResponse, CoreError> {
    let description = service_info(location, service_id).await?;
    let declaration = description
        .actions
        .iter()
        .find(|item| item.name == action && item.name.starts_with("Get"))
        .ok_or("请选择设备提供的查询动作")?;
    let inputs: Vec<_> = declaration
        .arguments
        .iter()
        .filter(|arg| arg.direction == "in")
        .collect();
    if args.len() != inputs.len() || inputs.iter().any(|arg| !args.contains_key(&arg.name)) {
        return Err("查询参数与设备声明不符".into());
    }
    if !valid_xml_name(action) || args.keys().any(|name| !valid_xml_name(name)) {
        return Err("动作或参数名称无效".into());
    }
    let ordered_args = inputs
        .iter()
        .map(|arg| (arg.name.clone(), args[&arg.name].clone()))
        .collect::<Vec<_>>();
    soap(&service(location, service_id).await?, action, &ordered_args).await
}

fn valid_xml_name(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic()
                || byte == b'_'
                || (index > 0 && (byte.is_ascii_digit() || byte == b'-' || byte == b'.'))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 设备描述决定实际请求地址；URLBase 和相对路径必须按设备声明解析。
    #[test]
    fn service_endpoints_follow_device_url_base() {
        let mut response = TestResponse { time: 0, url: "http://192.168.1.2/description/root.xml".into(), request: String::new(), response_headers: String::new(), response_url: None, status: Some(200), elapsed_ms: 0, fields: BTreeMap::new(), error: None,
            raw: r#"<root xmlns="urn:schemas-upnp-org:device-1-0"><URLBase>http://192.168.1.2:1234/upnp/</URLBase><device><friendlyName>电视</friendlyName><serviceList><service><serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType><serviceId>transport</serviceId><SCPDURL>service.xml</SCPDURL><controlURL>/control</controlURL></service></serviceList></device></root>"#.into() };
        let services = parse_device(&mut response).unwrap();
        assert_eq!(response.fields["friendlyName"], "电视");
        assert_eq!(
            services[0].scpd_url,
            "http://192.168.1.2:1234/upnp/service.xml"
        );
        assert_eq!(services[0].control_url, "http://192.168.1.2:1234/control");
    }
}
