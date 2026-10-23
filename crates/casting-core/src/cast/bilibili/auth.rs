use super::{
    client::{PASSPORT_HOST, base_params, bili_client, sign_params},
    session::{BilibiliSession, now_ts},
};
use serde_json::Value;

/// 扫码登录并返回会话，保存由调用方决定。`on_qr(url)` 在获取二维码后调用一次。
pub async fn login_qr(
    buvid: &str,
    on_qr: impl Fn(String) + Send + Sync,
) -> Result<BilibiliSession, String> {
    let client = bili_client();
    let params = base_params(buvid);
    let body = sign_params(&params);

    // 获取二维码和授权码
    let json: Value = client
        .post(format!("{}/x/passport-tv-login/qrcode/auth_code", PASSPORT_HOST))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("auth_code: {}", e))?
        .json()
        .await
        .map_err(|e| format!("auth_code parse: {}", e))?;

    if json["code"].as_i64() != Some(0) {
        return Err(format!("auth_code error: {}", json["message"]));
    }
    let qr_url = json["data"]["url"].as_str().ok_or("no url")?.to_string();
    let auth_code = json["data"]["auth_code"].as_str().ok_or("no auth_code")?.to_string();

    on_qr(qr_url);

    loop {
        // 等待 2 秒后轮询授权码状态
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let mut poll_params = base_params(buvid);
        poll_params.push(("auth_code".into(), auth_code.clone()));
        let poll_body = sign_params(&poll_params);

        // 轮询授权码状态
        let poll: Value = client
            .post(format!("{}/x/passport-tv-login/qrcode/poll", PASSPORT_HOST))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(poll_body)
            .send()
            .await
            .map_err(|e| format!("poll: {}", e))?
            .json()
            .await
            .map_err(|e| format!("poll parse: {}", e))?;

        // 处理轮询结果
        match poll["code"].as_i64() {
            // 成功获取会话
            Some(0) => {
                let token = poll["data"]["token_info"]["access_token"]
                    .as_str()
                    .ok_or("no access_token")?
                    .to_string();
                let mid = poll["data"]["mid"].as_u64().ok_or("no mid")?;
                let expires_in = poll["data"]["token_info"]["expires_in"]
                    .as_u64()
                    .unwrap_or(7776000);
                let expires_at = Some(now_ts() + expires_in);
                let session = BilibiliSession { access_token: token, mid, expires_at };
                return Ok(session);
            }
            // 授权码过期
            Some(86038) => return Err("QR code expired".into()),
            _ => log::debug!("bili poll: {} {}", poll["code"], poll["message"]),
        }
    }
}
