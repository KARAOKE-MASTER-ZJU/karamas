//! 实机检查广播回传。显式指定会话文件和曲目，仅在唯一在线设备上播放。
use karamas_core::cast::{
    Caster, SongRef,
    bilibili::{BilibiliCaster, BilibiliSession, list_devices},
};
use std::{path::PathBuf, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .filter_module("BILI_BROADCAST", log::LevelFilter::Debug)
        .init();
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(
        args.next()
            .ok_or("用法：bilibili_broadcast_probe <会话文件> <BV号-page0>")?,
    );
    let song = args
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or("请指定测试曲目")?;
    let sender_buvid = std::env::var("BILIBILI_BUVID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or("请通过 BILIBILI_BUVID 配置发送端设备标识")?;
    let session: BilibiliSession = serde_json::from_slice(&std::fs::read(path)?)?;
    let devices = list_devices(&session)
        .await
        .map_err(|error| format!("查询设备失败：{error}"))?;
    if devices.len() != 1 {
        return Err(format!("需要唯一在线设备，当前找到 {} 台", devices.len()).into());
    }
    log::info!("实机验证设备：{}，测试曲目：{}", devices[0].name, song);
    let caster = BilibiliCaster::new(session, devices[0].buvid.clone(), sender_buvid);
    caster.start_broadcast();
    caster.play_song(&SongRef(song)).await?;
    log::info!("播放命令成功，监听 40 秒；请检查 BILI_BROADCAST 的电视状态回传日志");
    tokio::time::sleep(Duration::from_secs(40)).await;
    caster.stop_status_subscription();
    tokio::time::sleep(Duration::from_millis(100)).await;
    Ok(())
}
