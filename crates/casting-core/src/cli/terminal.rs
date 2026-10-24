use anyhow::Result;
use crossterm::{event, terminal};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};
use ktv_casting_lib::{
    ENGINE_STATE, cycle_quality_core, toggle_danmaku_core, toggle_pause_core, trigger_next_song,
    trigger_prev_song, volume_down_core, volume_up_core,
};
use log::{Log, Metadata, Record, info, warn};
use std::{fmt::Write, time::Duration};

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

const VOLUME_STEP: u32 = 5;

/// 负责监听键盘输入，并在后台线程中处理快捷键事件
pub(super) fn spawn_keyboard_handler() {
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
                            match trigger_prev_song().await {
                                Ok(()) => info!("⏮ 上一首"),
                                Err(e) => warn!("切上一首失败: {}", e),
                            }
                        }
                        event::KeyCode::Char('n') => {
                            match trigger_next_song().await {
                                Ok(()) => info!("⏭ 下一首"),
                                Err(e) => warn!("切下一首失败: {}", e),
                            }
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

/// 设置进度条样式
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


pub(super) fn setup_env() -> ProgressBar {
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

/// 读取核心库缓存的进度，通过回调更新 UI。
pub(super) async fn run_cli_monitor<FL, FP>(mut set_len: FL, mut set_pos: FP) -> Result<()>
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

        let (current, total) = ktv_casting_lib::get_current_progress().await;
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
