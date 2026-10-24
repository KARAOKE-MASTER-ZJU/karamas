use crate::error::{CommandError, CommandResult};
use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomEntryMode {
    Create,
    Join,
}

#[derive(Deserialize)]
struct RoomResponse {
    exists: Option<bool>,
    success: Option<bool>,
    msg: Option<String>,
}

/// 沿用点歌台的创建、查询接口，成功后由前端保存房间配置。
#[tauri::command]
pub async fn enter_room<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    server_url: String,
    room_id: String,
    mode: RoomEntryMode,
) -> CommandResult<()> {
    let mut url = super::session::http_url(&server_url)?;
    if url.query().is_some() || url.fragment().is_some() || room_id.trim().is_empty() {
        return Err(CommandError::new(
            "INVALID_ARGUMENT",
            "服务器地址不能带查询参数或片段，房间号不能为空",
        ));
    }
    crate::platform::ensure_room_access(&app, &url).await?;
    let endpoint = match mode {
        RoomEntryMode::Create => "createRoom",
        RoomEntryMode::Join => "roomExists",
    };
    url.set_path(&format!(
        "{}/api/{endpoint}",
        url.path().trim_end_matches('/')
    ));
    url.query_pairs_mut().append_pair("roomId", room_id.trim());
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| CommandError::new("NETWORK_ERROR", e.to_string()))?;
    let request = match mode {
        RoomEntryMode::Create => client.post(url),
        RoomEntryMode::Join => client.get(url),
    };
    let response = request
        .send()
        .await
        .map_err(|e| CommandError::new("NETWORK_ERROR", format!("无法连接点歌台：{e}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(CommandError::new(
            "ROOM_ERROR",
            format!("点歌台请求失败：HTTP {status}"),
        ));
    }
    let body: RoomResponse = response
        .json()
        .await
        .map_err(|_| CommandError::new("ROOM_ERROR", "点歌台返回了无效的房间信息"))?;
    match mode {
        RoomEntryMode::Join => match body.exists {
            Some(true) => Ok(()),
            Some(false) => Err(CommandError::new(
                "ROOM_NOT_FOUND",
                "房间不存在，请先创建房间",
            )),
            None => Err(CommandError::new(
                "ROOM_ERROR",
                "服务器不支持房间查询接口，请更新服务器",
            )),
        },
        RoomEntryMode::Create => {
            if body.success == Some(true) {
                Ok(())
            } else {
                let message = body
                    .msg
                    .unwrap_or_else(|| "服务器不支持房间创建接口，请更新服务器".into());
                let message = if message == "房间已存在" {
                    "房间号已被占用，请更换房间号或选择加入房间".into()
                } else {
                    message
                };
                Err(CommandError::new("ROOM_ERROR", message))
            }
        }
    }
}
