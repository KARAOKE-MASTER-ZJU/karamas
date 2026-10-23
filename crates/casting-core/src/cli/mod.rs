//! 命令行入口，组合交互设置和终端控制。
mod setup;
mod terminal;

use anyhow::Result;
use indicatif::ProgressDrawTarget;
use ktv_casting_lib::cast::dlna::DlnaController;
use ktv_casting_lib::{start_bilibili_engine_core, start_engine_core};
use log::info;
use setup::{
    acquire_bilibili_session, get_room_config_interactively, select_bilibili_device_interactively,
    select_dlna_device_interactively,
};
use std::io;
use terminal::{run_cli_monitor, setup_env, spawn_keyboard_handler};

pub async fn run() -> Result<()> {
    let pb = setup_env();

    // 1. 选择投屏模式
    let (base_url, room_id) = get_room_config_interactively()?;

    println!("选择投屏模式：");
    println!("  1. DLNA（局域网，需要设备支持）");
    println!("  2. 哔哩哔哩投屏（需要登录B站账号）");
    print!("请选择 [1/2，默认1]：");
    io::Write::flush(&mut io::stdout())?;
    let mut mode_input = String::new();
    io::stdin().read_line(&mut mode_input)?;

    if mode_input.trim() == "2" {
        // --- Bilibili 模式 ---
        let sender_buvid = std::env::var("BILIBILI_BUVID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("请通过 BILIBILI_BUVID 配置发送端设备标识"))?;
        let session = acquire_bilibili_session(&sender_buvid).await?;
        info!("登录成功，UID: {}", session.mid);
        let device_buvid = select_bilibili_device_interactively(&session).await?;
        start_bilibili_engine_core(
            base_url,
            room_id,
            session,
            device_buvid,
            sender_buvid,
            tokio::runtime::Handle::current(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    } else {
        // --- DLNA 模式 ---
        let controller = DlnaController::new();
        let device = select_dlna_device_interactively(&controller).await?;
        start_engine_core(
            base_url,
            room_id,
            device.location.clone(),
            tokio::runtime::Handle::current(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    }

    pb.set_draw_target(ProgressDrawTarget::stdout());
    println!("─────────────────────────────────────────");
    println!("  p      暂停 / 继续");
    println!("  b      切上一首");
    println!("  n      切下一首");
    println!("  + / =  音量 +5");
    println!("  -      音量 -5");
    println!("  d      弹幕开关（仅B站投屏）");
    println!("  q      切换清晰度（仅B站投屏）");
    println!("  Ctrl-C 退出");
    println!("─────────────────────────────────────────");
    spawn_keyboard_handler();

    let pb_for_len = pb.clone();
    let pb_for_pos = pb.clone();
    run_cli_monitor(
        move |total| pb_for_len.set_length(total),
        move |curr| pb_for_pos.set_position(curr),
    )
    .await?;

    Ok(())
}
