//! 本地播放器契约。宿主传递小消息，媒体数据由回环 HTTP 服务提供。
use super::{
    Capabilities, CastError, Caster, MediaFormat, PlaybackSnapshot, Progress, Quality, SongRef,
};
use crate::media::session::MediaSessions;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex as AsyncMutex, oneshot};

#[derive(Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LocalAction {
    Loading,
    Load {
        url: String,
        position: u32,
        playing: bool,
        volume: u32,
        muted: bool,
    },
    Play,
    Pause,
    Seek {
        seconds: u32,
    },
    Volume {
        value: u32,
    },
    Stop,
    Error {
        message: String,
    },
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalCommand {
    pub session_id: String,
    pub command_id: u64,
    pub action: LocalAction,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalReport {
    pub session_id: String,
    pub command_id: u64,
    pub current_secs: f64,
    pub total_secs: f64,
    pub paused: bool,
    /// 媒体正在定位，或用户尚未松开播放器中的拖动操作。
    pub seeking: bool,
    pub muted: bool,
    pub ended: bool,
    pub volume: u32,
    /// 操作完成时必须回传确认；周期状态不能替代操作确认。
    pub acknowledged: bool,
    pub error: Option<String>,
}

pub type LocalCommandSink = Arc<dyn Fn(LocalCommand) -> Result<(), String> + Send + Sync>;

struct State {
    command_id: u64,
    pending: Option<oneshot::Sender<Result<(), String>>>,
    report: Option<(LocalReport, Instant)>,
    stopped: bool,
    song: Option<String>,
    quality: Quality,
    format: MediaFormat,
    actual_quality: Option<(Quality, MediaFormat)>,
    volume: u32,
    muted: bool,
}

pub struct LocalCaster {
    session_id: String,
    port: u16,
    sessions: Arc<MediaSessions>,
    send: LocalCommandSink,
    state: Mutex<State>,
    control: AsyncMutex<()>,
    client: reqwest::Client,
}

impl LocalCaster {
    pub fn new(port: u16, sessions: Arc<MediaSessions>, send: LocalCommandSink) -> Self {
        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            port,
            sessions,
            send,
            state: Mutex::new(State {
                command_id: 0,
                pending: None,
                report: None,
                stopped: false,
                song: None,
                quality: Quality::P720,
                format: MediaFormat::Mp4,
                actual_quality: None,
                volume: 100,
                muted: false,
            }),
            control: AsyncMutex::new(()),
            client: reqwest::Client::builder()
                .connect_timeout(crate::config::PROXY_CONNECT_TIMEOUT)
                .build()
                .expect("valid media client"),
        }
    }

    async fn command(&self, action: LocalAction) -> Result<(), CastError> {
        let (tx, rx) = oneshot::channel();
        let command = {
            let mut state = self.state.lock().unwrap();
            if state.stopped {
                return Err(CastError::Device("本地播放会话已关闭".into()));
            }
            state.command_id += 1;
            state.pending = Some(tx);
            LocalCommand {
                session_id: self.session_id.clone(),
                command_id: state.command_id,
                action,
            }
        };
        let mut timed_out = false;
        let result = match (self.send)(command) {
            Err(error) => Err(error),
            Ok(()) => match tokio::time::timeout(Duration::from_secs(30), rx).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err("本地播放器已断开".into()),
                Err(_) => {
                    timed_out = true;
                    Err("本地播放器响应超时，请重试".into())
                }
            },
        };
        let mut state = self.state.lock().unwrap();
        state.pending = None;
        if result.is_err() {
            state.report = None;
        }
        let cancellation = if timed_out && !state.stopped {
            state.command_id += 1;
            Some(LocalCommand {
                session_id: self.session_id.clone(),
                command_id: state.command_id,
                action: LocalAction::Error {
                    message: "本地播放器响应超时，请重新播放。".into(),
                },
            })
        } else {
            None
        };
        drop(state);
        if let Some(command) = cancellation {
            let _ = (self.send)(command);
        }
        result.map_err(CastError::Device)
    }

    async fn load(&self, song: &str, position: u32, playing: bool) -> Result<(), CastError> {
        let (quality, format, volume, muted) = {
            let mut state = self.state.lock().unwrap();
            state.song = Some(song.to_owned());
            state.report = None;
            (state.quality, state.format, state.volume, state.muted)
        };
        self.command(LocalAction::Loading).await?;
        let session = self.sessions.activate_with_format(song, quality, format);
        if let Err(error) = session.prepare(&self.client).await {
            let message = format!("准备本地视频失败：{error}");
            let _ = self
                .command(LocalAction::Error {
                    message: message.clone(),
                })
                .await;
            return Err(CastError::Device(message));
        }
        self.command(LocalAction::Load {
            url: format!("http://127.0.0.1:{}/{}", self.port, session.path()),
            position,
            playing,
            volume,
            muted,
        })
        .await?;
        self.state.lock().unwrap().actual_quality = Some(session.resolved_quality());
        Ok(())
    }

    fn fresh_report(state: &State) -> Option<&LocalReport> {
        state
            .report
            .as_ref()
            .filter(|(report, at)| {
                !state.stopped
                    && at.elapsed() < Duration::from_secs(5)
                    && report.command_id == state.command_id
                    && report.error.is_none()
            })
            .map(|(report, _)| report)
    }
}

#[async_trait]
impl Caster for LocalCaster {
    fn kind(&self) -> crate::cast::CasterKind {
        crate::cast::CasterKind::Local
    }

    fn stop_status_subscription(&self) {
        let command = {
            let mut state = self.state.lock().unwrap();
            state.stopped = true;
            state.report = None;
            if let Some(tx) = state.pending.take() {
                let _ = tx.send(Err("本地播放会话已关闭".into()));
            }
            state.command_id += 1;
            LocalCommand {
                session_id: self.session_id.clone(),
                command_id: state.command_id,
                action: LocalAction::Stop,
            }
        };
        let _ = (self.send)(command);
        self.sessions.clear();
    }

    fn report_local_playback(&self, report: LocalReport) -> Result<bool, CastError> {
        let mut state = self.state.lock().unwrap();
        if state.stopped
            || report.session_id != self.session_id
            || report.command_id != state.command_id
        {
            return Ok(false);
        }
        if !report.current_secs.is_finite()
            || !report.total_secs.is_finite()
            || report.current_secs < 0.0
            || report.total_secs < 0.0
            || report.current_secs > i32::MAX as f64
            || report.total_secs > i32::MAX as f64
            || report.volume > 100
        {
            return Err(CastError::Device("本地播放状态无效".into()));
        }
        if report.acknowledged {
            if let Some(tx) = state.pending.take() {
                let _ = tx.send(report.error.clone().map_or(Ok(()), Err));
            }
        }
        state.volume = report.volume;
        state.muted = report.muted;
        state.report = Some((report, Instant::now()));
        Ok(true)
    }

    fn playback_snapshot(&self) -> Option<PlaybackSnapshot> {
        let state = self.state.lock().unwrap();
        Some(match Self::fresh_report(&state) {
            Some(report) => PlaybackSnapshot {
                progress: Progress {
                    current_secs: report.current_secs as u32,
                    total_secs: report.total_secs as u32,
                },
                is_playing: !report.paused && !report.ended,
                ended: Some(report.ended),
            },
            None => PlaybackSnapshot {
                progress: Progress::default(),
                is_playing: false,
                ended: None,
            },
        })
    }

    fn can_auto_advance(&self) -> bool {
        let state = self.state.lock().unwrap();
        Self::fresh_report(&state).is_some_and(|report| !report.seeking)
    }

    async fn play_song(&self, song: &SongRef) -> Result<(), CastError> {
        let _control = self.control.lock().await;
        self.load(&song.0, 0, true).await
    }
    async fn resume(&self) -> Result<(), CastError> {
        let _control = self.control.lock().await;
        self.command(LocalAction::Play).await
    }
    async fn pause(&self) -> Result<(), CastError> {
        let _control = self.control.lock().await;
        self.command(LocalAction::Pause).await
    }
    async fn stop(&self) -> Result<(), CastError> {
        let _control = self.control.lock().await;
        if self.state.lock().unwrap().song.take().is_none() {
            return Ok(());
        }
        self.sessions.clear();
        self.command(LocalAction::Stop).await
    }
    async fn seek(&self, seconds: u32) -> Result<(), CastError> {
        let _control = self.control.lock().await;
        self.command(LocalAction::Seek { seconds }).await
    }
    async fn get_progress(&self) -> Result<Progress, CastError> {
        Ok(self.playback_snapshot().unwrap().progress)
    }
    async fn set_volume(&self, value: u32) -> Result<(), CastError> {
        let _control = self.control.lock().await;
        self.command(LocalAction::Volume {
            value: value.min(100),
        })
        .await
    }
    async fn get_volume(&self) -> Result<Option<u32>, CastError> {
        let state = self.state.lock().unwrap();
        Ok(Some(if state.muted { 0 } else { state.volume }))
    }
    async fn set_media_quality(
        &self,
        quality: Quality,
        format: MediaFormat,
    ) -> Result<(), CastError> {
        let _control = self.control.lock().await;
        let previous = {
            let state = self.state.lock().unwrap();
            (state.quality, state.format)
        };
        let (song, position, playing) = {
            let mut state = self.state.lock().unwrap();
            state.quality = quality;
            state.format = format;
            let report = Self::fresh_report(&state);
            (
                state.song.clone(),
                report.map_or(0, |r| r.current_secs as u32),
                report.is_some_and(|r| !r.paused && !r.ended),
            )
        };
        if let Some(song) = song {
            if let Err(error) = self.load(&song, position, playing).await {
                let mut state = self.state.lock().unwrap();
                (state.quality, state.format) = previous;
                return Err(error);
            }
        }
        Ok(())
    }
    fn get_quality(&self) -> Result<Quality, CastError> {
        let state = self.state.lock().unwrap();
        Ok(state
            .actual_quality
            .map_or(state.quality, |actual| actual.0))
    }
    fn get_media_format(&self) -> Option<MediaFormat> {
        let state = self.state.lock().unwrap();
        Some(state.actual_quality.map_or(state.format, |actual| actual.1))
    }
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
    use super::*;

    fn player() -> (
        Arc<LocalCaster>,
        tokio::sync::mpsc::UnboundedReceiver<LocalCommand>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Arc::new(LocalCaster::new(
                12345,
                Arc::new(MediaSessions::default()),
                Arc::new(move |command| tx.send(command).map_err(|e| e.to_string())),
            )),
            rx,
        )
    }
    fn report(command: &LocalCommand, acknowledged: bool) -> LocalReport {
        LocalReport {
            session_id: command.session_id.clone(),
            command_id: command.command_id,
            current_secs: 20.0,
            total_secs: 90.0,
            paused: false,
            seeking: false,
            muted: false,
            ended: false,
            volume: 50,
            acknowledged,
            error: None,
        }
    }

    // 自动播放被拒绝等播放器错误必须使控制请求失败，不能被当成播放成功。
    #[tokio::test]
    async fn rejected_playback_does_not_report_success() {
        let (player, mut commands) = player();
        let task = tokio::spawn({
            let p = player.clone();
            async move { p.resume().await }
        });
        let command = commands.recv().await.unwrap();
        let mut rejected = report(&command, true);
        rejected.error = Some("播放被拒绝".into());
        player.report_local_playback(rejected).unwrap();
        assert!(task.await.unwrap().is_err());
        assert!(!player.playback_snapshot().unwrap().is_playing);
    }

    // 原生静音只改变有效音量；重新加载媒体时仍需保留滑块音量供取消静音恢复。
    #[tokio::test]
    async fn native_mute_preserves_volume_for_media_reload() {
        let (player, mut commands) = player();
        let task = tokio::spawn({
            let p = player.clone();
            async move { p.resume().await }
        });
        let command = commands.recv().await.unwrap();
        player
            .report_local_playback(report(&command, true))
            .unwrap();
        task.await.unwrap().unwrap();
        let mut muted = report(&command, false);
        muted.volume = 73;
        muted.muted = true;
        player.report_local_playback(muted).unwrap();
        assert_eq!(player.get_volume().await.unwrap(), Some(0));
        let task = tokio::spawn({
            let p = player.clone();
            async move {
                p.play_song(&SongRef("http://example.invalid/video.mp4".into()))
                    .await
            }
        });
        let loading = commands.recv().await.unwrap();
        let mut loading_ack = report(&loading, true);
        loading_ack.volume = 73;
        loading_ack.muted = true;
        player.report_local_playback(loading_ack).unwrap();
        let loaded = commands.recv().await.unwrap();
        assert!(matches!(
            loaded.action,
            LocalAction::Load {
                volume: 73,
                muted: true,
                ..
            }
        ));
        let mut loaded_ack = report(&loaded, true);
        loaded_ack.volume = 73;
        loaded_ack.muted = true;
        player.report_local_playback(loaded_ack).unwrap();
        task.await.unwrap().unwrap();
        let mut restored = report(&loaded, false);
        restored.volume = 73;
        player.report_local_playback(restored).unwrap();
        assert_eq!(player.get_volume().await.unwrap(), Some(73));
    }
}
