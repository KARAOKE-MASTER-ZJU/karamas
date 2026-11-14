//! 本地播放器契约。宿主传递小消息，媒体数据由回环 HTTP 服务提供。
use super::{
    Capabilities, CastError, Caster, MediaFormat, PlaybackSnapshot, Progress, Quality, SongRef,
};
use crate::media::session::{MediaSessions, PlaybackMediaType};
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
        media_type: PlaybackMediaType,
        filesize: Option<u64>,
        live: bool,
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
    pub live: bool,
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
    live: bool,
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
                live: false,
                quality: Quality::P720,
                format: MediaFormat::Mp4,
                actual_quality: None,
                volume: 100,
                muted: false,
            }),
            control: AsyncMutex::new(()),
            client: crate::media::upstream::client().expect("valid media client"),
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
            state.live = false;
            state.report = None;
            (state.quality, state.format, state.volume, state.muted)
        };
        self.command(LocalAction::Loading).await?;
        let session = self
            .sessions
            .activate_local_with_format(song, quality, format);
        let media = match session.prepare(&self.client).await {
            Ok(media) => media,
            Err(error) => {
                let message = format!("准备本地视频失败：{error}");
                let _ = self
                    .command(LocalAction::Error {
                        message: message.clone(),
                    })
                    .await;
                return Err(CastError::Device(message));
            }
        };
        let live = session.is_live();
        self.state.lock().unwrap().live = live;
        self.command(LocalAction::Load {
            url: format!(
                "http://127.0.0.1:{}/{}",
                self.port,
                self.sessions.proxy_path(&session.path()),
            ),
            media_type: media.playback_type(),
            filesize: match media.as_ref() {
                crate::media::session::SessionMedia::Flv { filesize, .. } => *filesize,
                _ => None,
            },
            live,
            position: if live { 0 } else { position },
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
        state.live = report.live;
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
        if !self.capabilities().seek {
            return Err(CastError::Device(
                "直播暂不支持调整进度，请使用重新播放回到直播".into(),
            ));
        }
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
    fn is_live(&self) -> bool {
        self.state.lock().unwrap().live
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            absolute_volume: true,
            seek: !self.is_live(),
            hardware_progress: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn media_server() -> (String, actix_web::dev::ServerHandle) {
        use actix_web::{App, HttpRequest, HttpResponse, HttpServer, web};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = HttpServer::new(|| {
            App::new().default_service(web::to(|request: HttpRequest| async move {
                if request.query_string() == "stream" {
                    use futures_util::StreamExt;
                    HttpResponse::Ok()
                        .content_type("application/octet-stream")
                        .streaming(
                            futures_util::stream::once(async {
                                Ok::<_, std::io::Error>(bytes::Bytes::from_static(
                                    b"FLV\x01\x05\x00\x00\x00\x09",
                                ))
                            })
                            .chain(futures_util::stream::pending()),
                        )
                } else if request.query_string() == "vod" {
                    HttpResponse::Ok()
                        .content_type("video/x-flv")
                        .body(&b"FLV\x01\x05\x00\x00\x00\x09"[..])
                } else {
                    HttpResponse::Ok()
                        .content_type("video/mp4")
                        .body(&b"\x00\x00\x00\x18ftypisom"[..])
                }
            }))
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .run();
        let handle = server.handle();
        actix_web::rt::spawn(server);
        (format!("http://{address}"), handle)
    }

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
            live: false,
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
    #[actix_web::test]
    async fn native_mute_preserves_volume_for_media_reload() {
        let (source, server) = media_server().await;
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
            async move { p.play_song(&SongRef(format!("{source}/video.mp4"))).await }
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
        server.stop(true).await;
    }

    // 格式不决定直播属性；播放器确认后再更新直播和定位能力。
    #[actix_web::test]
    async fn local_media_load_preserves_live_and_seek_contracts() {
        let (source, server) = media_server().await;
        for (path, flv, live, length) in [
            ("/stream?stream", true, true, None),
            ("/video.mp4?vod", true, false, Some(9)),
            ("/video.flv", false, false, None),
        ] {
            let (player, mut commands) = player();
            let task = tokio::spawn({
                let player = player.clone();
                let source = source.clone();
                async move { player.load(&format!("{source}{path}"), 42, true).await }
            });
            let loading = commands.recv().await.unwrap();
            player
                .report_local_playback(report(&loading, true))
                .unwrap();
            let loaded = commands.recv().await.unwrap();
            let LocalAction::Load {
                media_type,
                filesize,
                live: actual_live,
                position,
                ..
            } = &loaded.action
            else {
                panic!("expected load command");
            };
            assert!(!*actual_live);
            assert_eq!(*filesize, length);
            assert_eq!(*position, 42);
            assert_eq!(
                *media_type,
                if flv {
                    PlaybackMediaType::Flv
                } else {
                    PlaybackMediaType::Native
                }
            );
            let mut playback = report(&loaded, true);
            playback.live = live;
            playback.total_secs = if live { 0.0 } else { 90.0 };
            player.report_local_playback(playback).unwrap();
            task.await.unwrap().unwrap();
            assert_eq!(player.is_live(), live);
            assert_eq!(player.capabilities().seek, !live);
        }
        server.stop(false).await;
    }
}
