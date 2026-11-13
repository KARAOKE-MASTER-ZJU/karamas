//! 测试媒体取源、代理和设备播放控制。
use super::{
    http::{TestResponse, http_url},
    upnp::{device_info, soap},
};
use crate::{
    CoreError,
    cast::{MediaFormat, Quality},
    media::{server, session::SessionMedia},
};
use serde::Deserialize;
use std::{sync::Mutex, time::Duration};

#[derive(Deserialize)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlaybackTest {
    Load {
        song_url: String,
        format: MediaFormat,
        direct: bool,
    },
    Play,
    Pause,
    Stop,
    Seek {
        seconds: u32,
    },
    Volume {
        volume: u32,
    },
    Read,
    ReadVolume,
}

pub struct PlaybackTester {
    pub location: String,
    server: Mutex<Option<server::DiagnosticServer>>,
}

impl PlaybackTester {
    pub fn new(location: String) -> Self {
        Self {
            location,
            server: Mutex::new(None),
        }
    }

    pub async fn execute(
        &self,
        test: PlaybackTest,
        runtime: tokio::runtime::Handle,
    ) -> Result<Vec<TestResponse>, CoreError> {
        let info = device_info(&self.location).await?;
        let rendering = matches!(test, PlaybackTest::Volume { .. } | PlaybackTest::ReadVolume);
        let kind = if rendering {
            ":RenderingControl:"
        } else {
            ":AVTransport:"
        };
        let service = info
            .services
            .iter()
            .find(|service| service.service_type.contains(kind))
            .ok_or_else(|| format!("设备未返回 {kind} 服务"))?;
        let mut args = vec![("InstanceID".to_owned(), "0".to_owned())];
        let action = match test {
            PlaybackTest::Load {
                song_url,
                format,
                direct,
            } => {
                http_url(&song_url)?;
                if direct && format == MediaFormat::Dash {
                    return Err("DASH 使用反向代理播放".into());
                }
                let url = http_url(&self.location)?;
                let ip = tokio::net::lookup_host((
                    url.host_str().ok_or("设备地址缺少主机名")?,
                    url.port_or_known_default().unwrap_or(80),
                ))
                .await?
                .find(|address| address.is_ipv4())
                .ok_or("设备地址没有 IPv4 地址")?
                .ip();
                let (server_ip, server_port, state) = {
                    let mut slot = self.server.lock().map_err(|_| "无法读取媒体服务")?;
                    if slot.is_none() {
                        *slot = Some(server::start_diagnostic(&ip.to_string(), runtime)?);
                    }
                    let server = slot.as_ref().ok_or("媒体服务未启动")?;
                    (server.ip, server.port, server.state.clone())
                };
                let sessions = &state.media_sessions;
                let session = if direct && format == MediaFormat::Hls {
                    sessions.activate_hls_with_format(&song_url, Quality::P720, format)
                } else {
                    sessions.activate_with_formats(&song_url, Quality::P720, format, &[format])
                };
                let upstream = crate::media::upstream::client()?;
                let media =
                    tokio::time::timeout(Duration::from_secs(65), session.prepare(&upstream))
                        .await
                        .map_err(|_| "准备测试媒体超时")??;
                let (quality, actual_format) = session.resolved_quality();
                let proxy_url = format!(
                    "http://{}:{}/{}",
                    server_ip,
                    server_port,
                    sessions.proxy_path(&session.path())
                );
                let media_url = if direct {
                    match media.as_ref() {
                        SessionMedia::Direct(url) => url.clone(),
                        SessionMedia::Hls(playlists) => {
                            format!("{proxy_url}|{}", playlists.request_headers())
                        }
                        _ => return Err("未取得所选模式的直连媒体".into()),
                    }
                } else {
                    proxy_url
                };
                session.warm_seek(0);
                let mut results = vec![soap(service, "Stop", &args).await?];
                let protocol = if actual_format == MediaFormat::Hls {
                    Some("http-get:*:application/vnd.apple.mpegurl:*")
                } else {
                    None
                };
                let metadata =
                    crate::cast::dlna::didl_lite_metadata("诊断测试", &media_url, protocol);
                args.push(("CurrentURI".into(), media_url));
                args.push(("CurrentURIMetaData".into(), metadata));
                results.push(soap(service, "SetAVTransportURI", &args).await?);
                results[1]
                    .fields
                    .insert("取源格式".into(), format!("{actual_format:?}"));
                results[1]
                    .fields
                    .insert("清晰度".into(), quality.label().into());
                if results[1].status == Some(200)
                    && results[1].error.is_none()
                    && !results[1].fields.contains_key("faultcode")
                {
                    let play_args = [
                        ("InstanceID".into(), "0".into()),
                        ("Speed".into(), "1".into()),
                    ];
                    results.push(soap(service, "Play", &play_args).await?);
                }
                return Ok(results);
            }
            PlaybackTest::Play => {
                args.push(("Speed".into(), "1".into()));
                "Play"
            }
            PlaybackTest::Pause => "Pause",
            PlaybackTest::Stop => "Stop",
            PlaybackTest::Seek { seconds } => {
                args.push(("Unit".into(), "REL_TIME".into()));
                args.push((
                    "Target".into(),
                    format!(
                        "{:02}:{:02}:{:02}",
                        seconds / 3600,
                        seconds / 60 % 60,
                        seconds % 60
                    ),
                ));
                "Seek"
            }
            PlaybackTest::Volume { volume } => {
                if volume > 100 {
                    return Err("音量范围为 0 到 100".into());
                }
                args.push(("Channel".into(), "Master".into()));
                args.push(("DesiredVolume".into(), volume.to_string()));
                "SetVolume"
            }
            PlaybackTest::Read => "GetPositionInfo",
            PlaybackTest::ReadVolume => {
                args.push(("Channel".into(), "Master".into()));
                "GetVolume"
            }
        };
        let response = soap(service, action, &args).await?;
        if action == "Stop" {
            self.server.lock().map_err(|_| "无法读取媒体服务")?.take();
        }
        Ok(vec![response])
    }
}
