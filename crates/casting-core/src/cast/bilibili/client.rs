use super::session::now_ts;
use std::sync::OnceLock;

pub(super) const APPKEY: &str = "4409e2ce8ffd12b8";
pub(super) const APP_SECRET: &str = "59b43e04ad6965f34319062b478f83dd";
pub(super) const DEVICE_ID: &str = "5R84Y6H5NB1VSDWTM9B2G56";
pub(super) const BUVID: &str = "CLI54H1B5DHS5GB4RS5GS5G6E";
pub(super) const UA: &str = "Mozilla/5.0 BiliTV/1.8.5 os/android model/OPD2404 mobi_app/android_tv_yst build/108500 channel/dangbei innerVer/108500 osVer/16 network/2";
pub(super) const PASSPORT_HOST: &str = "https://passport.snm0516.aisee.tv";
pub(super) const API_HOST: &str = "https://api.bilibili.com";

pub(super) fn base_params() -> Vec<(String, String)> {
    vec![
        ("appkey".into(), APPKEY.into()),
        ("bili_local_id".into(), DEVICE_ID.into()),
        ("build".into(), "108500".into()),
        ("buvid".into(), BUVID.into()),
        ("channel".into(), "dangbei".into()),
        ("device_id".into(), DEVICE_ID.into()),
        ("extend".into(), r#"{"option":"4"}"#.into()),
        ("local_id".into(), BUVID.into()),
        ("mobi_app".into(), "android_tv_yst".into()),
        ("platform".into(), "android".into()),
        ("statistics".into(), r#"{"appId":"18","platform":"3","version":"108500"}"#.into()),
        ("ts".into(), now_ts().to_string()),
    ]
}

pub(super) fn sign_params(params: &[(String, String)]) -> String {
    let mut sorted = params.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let parts: Vec<String> = sorted
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect();
    let body = parts.join("&");
    let digest = md5::compute(format!("{}{}", body, APP_SECRET));
    format!("{}&sign={:x}", body, digest)
}

pub(super) fn form_body(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

pub(super) fn bili_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(UA)
            .timeout(crate::config::API_TIMEOUT)
            .build()
            .expect("reqwest client")
    })
}
