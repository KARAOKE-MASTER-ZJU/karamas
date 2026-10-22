#[cfg(not(target_os = "android"))]
mod cli {
    use anyhow::{Context, Result, bail};
    use crossterm::event::{self};
    use crossterm::terminal;
    use indicatif::{ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};
    use ktv_casting_lib::cast::bilibili_caster::{self as bili, BilibiliSession};
    use ktv_casting_lib::dlna_controller::{DlnaController, DlnaDevice};
    use ktv_casting_lib::{ENGINE_STATE, cycle_quality_core, start_bilibili_engine_core, start_engine_core, toggle_danmaku_core, toggle_pause_core, trigger_next_song, trigger_prev_song, volume_down_core, volume_up_core};
    use log::{Log, Metadata, Record, info, warn};
    use std::fmt::Write;
    use std::io;
    use std::time::Duration;
    use url::Url;

    struct ProgressLogger {
        inner: Box<dyn Log>,
        pb: ProgressBar,
    }

    impl Log for ProgressLogger {
        fn enabled(&self, metadata: &Metadata) -> bool {
            self.inner.enabled(metadata)
        }
        fn log(&self, record: &Record) {
            if self.enabled(record.metadata()) {
                self.pb.suspend(|| {
                    self.inner.log(record);
                });
            }
        }
        fn flush(&self) {
            self.inner.flush();
        }
    }

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
            // 先完成所有可能失败的操作，再创建 Runtime，避免在 async 上下文里 drop Runtime
            let session = acquire_bilibili_session().await?;
            info!("登录成功，UID: {}", session.mid);
            let device_buvid = select_bilibili_device_interactively(&session).await?;
            let engine_rt = tokio::runtime::Runtime::new().context("Failed to create engine runtime")?;
            start_bilibili_engine_core(base_url, room_id, session, device_buvid, engine_rt)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
        } else {
            // --- DLNA 模式 ---
            let controller = DlnaController::new();
            let device = select_dlna_device_interactively(&controller).await?;
            let engine_rt = tokio::runtime::Runtime::new().context("Failed to create engine runtime")?;
            start_engine_core(base_url, room_id, device.location.clone(), engine_rt)
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

    async fn acquire_bilibili_session() -> Result<BilibiliSession> {
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
        Ok(session)
    }

    async fn select_bilibili_device_interactively(session: &BilibiliSession) -> Result<String> {
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
    async fn select_dlna_device_interactively(controller: &DlnaController) -> Result<DlnaDevice> {
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

    const VOLUME_STEP: u32 = 5;

    fn spawn_keyboard_handler() {
        let _ = terminal::enable_raw_mode();
        tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Handle::current();
            loop {
                if let Ok(event::Event::Key(key)) = event::read() {
                    if key.code == event::KeyCode::Char('c')
                        && key.modifiers.contains(event::KeyModifiers::CONTROL)
                    {
                        let _ = terminal::disable_raw_mode();
                        std::process::exit(0);
                    }

                    if key.kind != event::KeyEventKind::Press {
                        continue;
                    }

                    rt.block_on(async {
                        match key.code {
                            event::KeyCode::Char('p') => {
                                match toggle_pause_core().await {
                                    Ok(state) => {
                                        info!(
                                            "{}",
                                            if state {
                                                "▶ 已恢复播放"
                                            } else {
                                                "⏸ 已暂停"
                                            }
                                        );
                                    }
                                    Err(e) => warn!("暂停/恢复失败: {}", e),
                                }
                            }
                            event::KeyCode::Char('b') => {
                                trigger_prev_song();
                                info!("⏮ 上一首");
                            }
                            event::KeyCode::Char('n') => {
                                trigger_next_song();
                                info!("⏭ 下一首");
                            }
                            event::KeyCode::Char('+') | event::KeyCode::Char('=') => {
                                match volume_up_core(VOLUME_STEP).await {
                                    Ok(()) => info!("🔊 音量 +"),
                                    Err(e) => info!("音量调节失败: {}", e),
                                }
                            }
                            event::KeyCode::Char('-') => {
                                match volume_down_core(VOLUME_STEP).await {
                                    Ok(()) => info!("🔉 音量 -"),
                                    Err(e) => info!("音量调节失败: {}", e),
                                }
                            }
                            event::KeyCode::Char('d') => {
                                match toggle_danmaku_core().await {
                                    Ok(true) => info!("💬 弹幕已开启"),
                                    Ok(false) => info!("💬 弹幕已关闭"),
                                    Err(e) => info!("弹幕切换失败: {}", e),
                                }
                            }
                            event::KeyCode::Char('q') => {
                                match cycle_quality_core().await {
                                    Ok(quality) => info!("🎬 清晰度已切换为 {}", quality.label()),
                                    Err(e) => info!("清晰度切换失败: {}", e),
                                }
                            }
                            _ => {}
                        }
                    });
                }
            }
        });
    }

    fn setup_pb_style(pb: &ProgressBar) {
        pb.set_style(
            ProgressStyle::with_template("{bar:40.green/blue} {my_pos} / {my_len}")
                .unwrap()
                .with_key("my_pos", |s: &ProgressState, w: &mut dyn Write| {
                    write!(w, "{:02}:{:02}", s.pos() / 60, s.pos() % 60).unwrap()
                })
                .with_key("my_len", |s: &ProgressState, w: &mut dyn Write| {
                    write!(
                        w,
                        "{:02}:{:02}",
                        s.len().unwrap_or(0) / 60,
                        s.len().unwrap_or(0) % 60
                    )
                    .unwrap()
                })
                .progress_chars("━━━"),
        );
        pb.set_draw_target(ProgressDrawTarget::hidden());
    }
    fn setup_env() -> ProgressBar {
        if std::env::var("RUST_LOG").is_err() {
            unsafe {
                std::env::set_var("RUST_LOG", "INFO");
            }
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        let pb = ProgressBar::new(0);
        setup_pb_style(&pb);

        let env_log = env_logger::Builder::from_default_env().build();
        let _ = log::set_boxed_logger(Box::new(ProgressLogger {
            inner: Box::new(env_log),
            pb: pb.clone(),
        }))
        .map(|()| log::set_max_level(log::LevelFilter::Debug));
        pb
    }

    fn get_room_config_interactively() -> Result<(String, String)> {
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

    /// 负责进度查询、自动切歌，并通过回调更新 UI
    async fn run_cli_monitor<FL, FP>(mut set_len: FL, mut set_pos: FP) -> Result<()>
    where
        FL: FnMut(u64),
        FP: FnMut(u64),
    {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;

            let ctx = {
                // 锁被污染（其他线程 panic）时恢复而非 panic 退出整个 CLI
                let guard = ENGINE_STATE.read().unwrap_or_else(|e| e.into_inner());
                guard.as_ref().cloned()
            };

            if ctx.is_none() { break; }

            let (current, total) = ktv_casting_lib::poll_playback_progress().await;
            if current >= 0 && total > 0 {
                set_len(total as u64);
                set_pos(current as u64);
            } else {
                // Hide stale progress while the renderer is changing songs.
                set_pos(0);
                set_len(0);
            }
        }
        Ok(())
    }
}

#[cfg(not(target_os = "android"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::run().await
}

#[cfg(target_os = "android")]
fn main() {}
