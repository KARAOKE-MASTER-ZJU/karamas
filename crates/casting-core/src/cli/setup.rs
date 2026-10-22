use anyhow::{Context, Result, bail};
use ktv_casting_lib::cast::{
    bilibili::{self as bili, BilibiliSession},
    dlna::{DlnaController, DlnaDevice},
};
use log::info;
use std::io;
use url::Url;

pub(super) async fn acquire_bilibili_session() -> Result<BilibiliSession> {
    if let Some(session) = bili::load_session() {
        println!("已读取保存的B站会话 (UID: {})", session.mid);
        return Ok(session);
    }
    println!("未找到保存的会话，启动扫码登录...");
    let session = bili::login_qr(|qr_url| {
        println!("\n请用B站App扫描二维码登录：");
        let _ = qr2term::print_qr(&qr_url);
        println!("等待扫码...");
    })
    .await
    .map_err(|e| anyhow::anyhow!("B站登录失败: {}", e))?;
    bili::save_session(&session).map_err(|e| anyhow::anyhow!("保存B站会话失败: {}", e))?;
    Ok(session)
}

pub(super) async fn select_bilibili_device_interactively(session: &BilibiliSession) -> Result<String> {
    let devices = bili::list_devices(session)
        .await
        .map_err(|e| anyhow::anyhow!("获取设备列表失败: {}", e))?;
    if devices.is_empty() {
        bail!("没有在线的投屏设备，请先在B站App上开始投屏");
    }
    println!("在线投屏设备：");
    for (i, d) in devices.iter().enumerate() {
        println!("  {}: {}", i + 1, d);
    }
    print!("选择设备编号：");
    io::Write::flush(&mut io::stdout())?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let idx: usize = input.trim().parse::<usize>().context("无效编号")? - 1;
    devices
        .into_iter()
        .nth(idx)
        .map(|d| d.buvid)
        .context("编号超出范围")
}

// --- 辅助逻辑函数 ---
pub(super) async fn select_dlna_device_interactively(controller: &DlnaController) -> Result<DlnaDevice> {
    println!("设备发现方式：");
    println!("  1. 自动搜索（SSDP 多播）");
    println!("  2. 直接输入 IP（适用于 WiFi 不支持多播的场景）");
    print!("请选择 [1/2，默认1]：");
    io::Write::flush(&mut io::stdout())?;
    let mut choice = String::new();
    io::stdin().read_line(&mut choice)?;

    if choice.trim() == "2" {
        return select_device_by_url(controller).await;
    }

    // 默认：多播搜索
    println!("正在搜索 DLNA 设备（约5秒）...");
    let devices = controller.discover_devices().await.unwrap_or_default();
    if devices.is_empty() {
        bail!("未发现任何 DLNA 设备，可尝试选择模式 2 直接输入 IP");
    }
    pick_from_device_list(devices)
}

async fn select_device_by_url(controller: &DlnaController) -> Result<DlnaDevice> {
    println!("请输入设备 IP 或描述文件完整 URL");
    println!("  示例 IP：  192.168.1.100");
    println!("  示例 URL： http://192.168.1.100:9958/bilibili/description.xml");
    print!("输入：");
    io::Write::flush(&mut io::stdout())?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let raw = input.trim();

    let url = if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else {
        // 只输入了 IP 或 IP:PORT，补全为 B站小电视默认路径
        let host = raw.trim_end_matches('/');
        format!("http://{}:9958/bilibili/description.xml", host)
    };

    println!("正在连接：{}", url);
    controller
        .get_device_from_url(&url)
        .await
        .with_context(|| format!("无法连接设备：{}", url))
}

fn pick_from_device_list(devices: Vec<DlnaDevice>) -> Result<DlnaDevice> {
    println!("发现以下设备：");
    for (i, d) in devices.iter().enumerate() {
        println!("  {}: {} ({})", i, d.friendly_name, d.location);
    }
    print!("输入设备编号：");
    io::Write::flush(&mut io::stdout())?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let idx: usize = input.trim().parse()?;
    devices.into_iter().nth(idx).context("编号无效")
}

pub(super) fn get_room_config_interactively() -> Result<(String, String)> {
    println!("=== KTV投屏DLNA应用启动 ===");
    println!("输入房间链接:");
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let url_str = input.trim();

    let mut normalized = url_str.to_string();
    if !normalized.contains("://") && !normalized.is_empty() {
        normalized = format!("https://{}", normalized);
    }

    let parsed = Url::parse(&normalized).with_context(|| format!("无法解析 URL: {}", normalized))?;

    // 提取 base_url (协议 + 域名 + 端口)
    let base_url = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));
    let base_url = if let Some(port) = parsed.port() {
        format!("{}:{}", base_url, port)
    } else {
        base_url
    };

    let room_str = parsed
        .query_pairs()
        .find(|(key, _)| key == "roomId")
        .map(|(_, v)| v.into_owned())
        .or_else(|| {
            parsed
                .path_segments()?
                .filter(|s| !s.is_empty())
                .last()
                .map(|s| s.to_string())
        })
        .with_context(|| "URL 中未找到房间号")?;

    info!("解析成功: BaseURL={}, RoomID={}", base_url, room_str);
    Ok((base_url, room_str))
}
