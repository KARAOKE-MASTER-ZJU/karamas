use async_trait::async_trait;
use std::net::IpAddr;

use crate::cast::dlna::controller::{DlnaController, DlnaDevice};
use crate::cast::{Capabilities, CastError, Caster, Progress, Quality, SongRef};

/// Dlna投屏器
pub struct DlnaCaster {
    controller: DlnaController,
    device: DlnaDevice,
    server_ip: IpAddr,
    server_port: u16,
    current_song: std::sync::Mutex<Option<String>>,
    sessions: Option<std::sync::Arc<crate::media::session::MediaSessions>>,
    media_client: reqwest::Client,
    transport: tokio::sync::Mutex<()>,
    seek_sequence: std::sync::atomic::AtomicU64,
    loaded_generation: std::sync::atomic::AtomicU64,
}

/// Dlna投屏器实现
impl DlnaCaster {
    pub fn new(
        controller: DlnaController,
        device: DlnaDevice,
        server_ip: IpAddr,
        server_port: u16,
    ) -> Self {
        // 初始化dlna投屏器
        Self {
            controller,
            device,
            server_ip,
            server_port,
            current_song: std::sync::Mutex::new(None),
            sessions: None,
            media_client: reqwest::Client::builder()
                .connect_timeout(crate::config::PROXY_CONNECT_TIMEOUT)
                .build()
                .expect("valid media client"),
            transport: tokio::sync::Mutex::new(()),
            seek_sequence: std::sync::atomic::AtomicU64::new(0),
            loaded_generation: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// 设置媒体会话
    pub fn with_sessions(
        mut self,
        sessions: std::sync::Arc<crate::media::session::MediaSessions>,
    ) -> Self {
        self.sessions = Some(sessions);
        self
    }

    /// 重新加载曲目，指定播放位置
    async fn reload_song_at(
        &self,
        song: &str,
        position: u32,
        reason: &str,
    ) -> Result<(), CastError> {
        let quality = crate::cast::dlna::settings::get_dlna_quality();
        let direct = crate::cast::dlna::settings::get_dlna_direct();
        let sequence = self.seek_sequence.load(std::sync::atomic::Ordering::Relaxed);
        let session = self
            .sessions
            .as_ref()
            .filter(|_| !direct)
            .map(|sessions| sessions.activate(song, quality));
        let media_url = if direct {
            resolve_direct_url(song, quality).await?
        } else if let Some(session) = &session {
            session
                .prepare(&self.media_client)
                .await
                .map_err(|error| self.failure("准备媒体", error))?;
            session.path()
        } else if quality == Quality::P1080 {
            with_start_offset(song, position)
        } else {
            song.to_owned()
        };
        let _transport = self.transport.lock().await;
        if direct && sequence != self.seek_sequence.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(CastError::Device("播放请求已过期".into()));
        }
        if session.as_ref().is_some_and(|s| s.is_stopped()) {
            return Err(CastError::Device("曲目准备已被新请求替代".into()));
        }
        let _ = self.controller.stop(&self.device).await;
        if session.as_ref().is_some_and(|s| s.is_stopped()) {
            return Err(CastError::Device("播放请求已过期".into()));
        }
        if direct {
            if let Some(sessions) = &self.sessions { sessions.clear(); }
            self.loaded_generation.store(0, std::sync::atomic::Ordering::Relaxed);
            self.controller.set_avtransport_url(&self.device, &media_url, "").await
                .map_err(|error| self.failure("设置直连媒体地址", error))?;
        } else {
            self.controller
            .set_avtransport_uri(
                &self.device,
                &media_url,
                "",
                self.server_ip,
                self.server_port,
            )
            .await
            .map_err(|error| self.failure("设置媒体地址", error))?;
        }
        if session.as_ref().is_some_and(|s| s.is_stopped()) {
            return Err(CastError::Device("播放请求已过期".into()));
        }
        self.controller.play_for_cast(&self.device).await
            .map_err(|error| self.failure("发送播放指令", error))?;
        if let Some(session) = &session {
            self.loaded_generation
                .store(session.generation, std::sync::atomic::Ordering::Relaxed);
        }

        if (direct || session.is_some() || quality == Quality::P720) && position > 0 {
            if let Err(error) = self.controller.seek(&self.device, position).await {
                log::warn!(target: "DLNA1080", "切换媒体后恢复进度失败: quality={}, position={}s, error={}", quality.label(), position, error);
                return Err(self.failure("恢复播放进度", error));
            }
        }
        log::info!(
            target: "DLNA1080",
            "DLNA 已重载媒体: reason={}, quality={}, start={}s",
            reason,
            quality.label(),
            position
        );
        Ok(())
    }

    fn failure(&self, step: &str, error: impl std::fmt::Display) -> CastError {
        let message = format!("[DLNA] {}失败：设备={}，设备地址={}，错误={}",
            step, self.device.friendly_name, self.device.location, error);
        log::warn!("{}", message);
        CastError::Device(message)
    }
}

/// 解析直连媒体链接，返回媒体地址或错误信息
async fn resolve_direct_url(song: &str, quality: Quality) -> Result<String, CastError> {
    if crate::sources::youtube::is_youtube_url(song) {
        return Err(CastError::Device("此媒体需要在线混流，请启用 DLNA 媒体代理".into()));
    }
    if song.starts_with("http://") || song.starts_with("https://") {
        return Ok(song.to_owned());
    }
    if quality == Quality::P1080 {
        return Err(CastError::Device("DLNA 直连模式不支持 B站 1080P".into()));
    }
    let path = song.split('?').next().unwrap_or(song);
    let (bvid, page) = path.split_once("-page").unwrap_or((path, "0"));
    if !bvid.starts_with("BV") || bvid.len() != 12 || !bvid.is_ascii() {
        return Err(CastError::Device("直连模式需要有效的媒体链接或 BV 号".into()));
    }
    let page = page.parse::<u32>().map_err(|_| CastError::Device("无效的 B站分P".into()))?;
    crate::sources::bilibili::get_bilibili_direct_link(bvid, Some(page)).await
        .map_err(CastError::Device)
}

/// 添加开始时间参数到媒体链接
fn with_start_offset(song: &str, position: u32) -> String {
    let (path, query) = song.split_once('?').unwrap_or((song, ""));
    let parameters: Vec<&str> = query
        .split('&')
        .filter(|parameter| !parameter.is_empty() && !parameter.starts_with("start="))
        .collect();
    let prefix = parameters.join("&");
    if prefix.is_empty() {
        format!("{path}?start={position}")
    } else {
        format!("{path}?{prefix}&start={position}")
    }
}

/// 实现 DlnaCaster 结构体的 Drop 特征
impl Drop for DlnaCaster {
    fn drop(&mut self) {
        if let Some(sessions) = &self.sessions {
            sessions.clear();
        }
    }
}

/// 实现 DlnaCaster 结构体的 Caster 特征
#[async_trait]
impl Caster for DlnaCaster {
    fn kind(&self) -> crate::cast::CasterKind {
        crate::cast::CasterKind::Dlna
    }

    /// 播放曲目
    async fn play_song(&self, song: &SongRef) -> Result<(), CastError> {
        if let Ok(mut current) = self.current_song.lock() {
            *current = Some(song.0.clone());
        }
        self.seek_sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.reload_song_at(&song.0, 0, "播放歌曲").await
    }

    /// 继续播放
    async fn resume(&self) -> Result<(), CastError> {
        let _transport = self.transport.lock().await;
        self.controller.play(&self.device).await
            .map_err(|error| self.failure("继续播放", error))
    }

    /// 暂停播放
    async fn pause(&self) -> Result<(), CastError> {
        let _transport = self.transport.lock().await;
        self.controller.pause(&self.device).await
            .map_err(|error| self.failure("暂停", error))
    }

    /// 停止播放
    async fn stop(&self) -> Result<(), CastError> {
        self.seek_sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _transport = self.transport.lock().await;
        self.controller.stop(&self.device).await
            .map_err(|error| self.failure("停止播放", error))
    }

    /// 播放指定位置
    async fn seek(&self, secs: u32) -> Result<(), CastError> {
        let loaded_generation = self.loaded_generation.load(std::sync::atomic::Ordering::Relaxed);
        if !crate::cast::dlna::settings::get_dlna_direct() && self.sessions.is_none() && crate::cast::dlna::settings::get_dlna_quality() == Quality::P1080 {
            let song = self
                .current_song
                .lock()
                .ok()
                .and_then(|current| current.clone())
                .ok_or_else(|| CastError::Device("DLNA 尚未加载歌曲，无法定位".to_string()))?;
            log::info!(target: "DLNA1080", "1080P 定位请求: {}s；通过 start 参数重开混流", secs);
            return self.reload_song_at(&song, secs, "1080P 定位").await;
        }
        let sequence = self
            .seek_sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        let _transport = self.transport.lock().await;
        if sequence
            != self
                .seek_sequence
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Ok(());
        }
        let mut old_streams = None;
        if let Some(sessions) = self.sessions.as_ref().filter(|_| loaded_generation != 0) {
            if let Some(session) = sessions
                .get(
                    self.loaded_generation
                        .load(std::sync::atomic::Ordering::Relaxed),
                )
            {
                old_streams = session.seek_stream_cutoff();
                session.warm_seek(secs);
            } else {
                return Err(CastError::Device("新曲目正在准备，尚不能定位".into()));
            }
        }
        let started = std::time::Instant::now();
        let result = self.controller.seek(&self.device, secs).await
            .map_err(|error| self.failure("定位", error));
        if result.is_ok() {
            if let Some((cache, cutoff)) = old_streams {
                // Cancel only streams present before the accepted command.
                // A fast renderer may already have opened its new Range GET.
                cache.retire_streams(cutoff);
            }
        }
        log::info!(target: "DLNA1080", "原生 DLNA 定位指令返回: target={}s, command_ms={}, success={}（不代表画面已恢复）", secs, started.elapsed().as_millis(), result.is_ok());
        result
    }

    /// 查询播放进度
    async fn get_progress(&self) -> Result<Progress, CastError> {
        self.controller
            .get_secs(&self.device)
            .await
            .map(|(curr, total)| Progress {
                current_secs: curr,
                total_secs: total,
            })
            .map_err(|error| self.failure("查询进度", error))
    }

    /// 设置音量
    async fn set_volume(&self, volume: u32) -> Result<(), CastError> {
        self.controller
            .set_volume(&self.device, volume.clamp(0, 100))
            .await
            .map_err(|error| self.failure("设置音量", error))
    }

    /// 查询音量
    async fn get_volume(&self) -> Result<Option<u32>, CastError> {
        self.controller
            .get_volume(&self.device)
            .await
            .map(Some)
            .map_err(|error| self.failure("查询音量", error))
    }

    /// 设置清晰度
    async fn set_quality(&self, quality: Quality) -> Result<(), CastError> {
        if crate::cast::dlna::settings::get_dlna_direct() && quality == Quality::P1080 {
            return Err(CastError::Device("DLNA 直连模式不支持 B站 1080P，请先启用媒体代理".into()));
        }
        let previous = crate::cast::dlna::settings::get_dlna_quality();
        if previous == quality {
            return Ok(());
        }
        log::info!(target: "DLNA1080", "切换 DLNA 清晰度: {} -> {}", previous.label(), quality.label());
        let position = self
            .controller
            .get_secs(&self.device)
            .await
            .ok()
            .map(|progress| progress.0)
            .unwrap_or(0);
        crate::cast::dlna::settings::set_dlna_quality(quality).map_err(|message| {
            log::error!(target: "DLNA1080", "拒绝 DLNA 清晰度切换: {}", message);
            CastError::Device(message.to_string())
        })?;

        // Re-open the same proxy URL so the renderer immediately requests the
        // newly selected representation. Keep the old position when possible.
        let song = self
            .current_song
            .lock()
            .ok()
            .and_then(|current| current.clone());
        if let Some(song) = song {
            self.reload_song_at(&song, position, "清晰度切换").await?;
        } else {
            log::info!(target: "DLNA1080", "DLNA 清晰度已保存，将在下一首生效: quality={}", quality.label());
        }
        Ok(())
    }

    /// 查询当前清晰度
    fn get_quality(&self) -> Result<Quality, CastError> {
        Ok(crate::cast::dlna::settings::get_dlna_quality())
    }

    /// 查询设备能力
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            absolute_volume: true,
            seek: true,
            hardware_progress: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{with_start_offset, resolve_direct_url};
    use super::Quality;

    #[tokio::test]
    async fn direct_url_preserves_signed_mp4_and_rejects_dash() {
        let url = "https://cdn.example/video.mp4?token=a%2Bb&expires=42";
        assert_eq!(resolve_direct_url(url, Quality::P720).await.unwrap(), url);
        assert!(resolve_direct_url("BV1dqj16aECG-page0", Quality::P1080).await.is_err());
        assert!(resolve_direct_url("https://www.youtube.com/watch?v=test", Quality::P720).await.is_err());
        assert!(resolve_direct_url("invalid-page0", Quality::P720).await.is_err());
    }

    #[test]
    fn replaces_an_existing_start_offset() {
        assert_eq!(
            with_start_offset("BV1xx-page0?qn=80&start=12", 42),
            "BV1xx-page0?qn=80&start=42"
        );
    }
}
