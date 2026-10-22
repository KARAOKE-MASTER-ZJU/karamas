//! Resolve a YouTube page to compatible MP4 DASH tracks on both desktop and Android.
//! Stream URLs are short-lived and must remain inside the current media session.
use innertube_rs::{Innertube, StreamingFormat, endpoints::player::resolve_stream_url_full};
use std::io;
use tokio::sync::OnceCell;
use url::Url;

static YOUTUBE_CLIENT: OnceCell<Innertube> = OnceCell::const_new();

pub struct YoutubeDash {
    pub video_url: String,
    pub audio_url: String,
    pub duration_secs: u32,
}

pub fn is_youtube_url(input: &str) -> bool {
    let Ok(url) = Url::parse(input) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && matches!(
            url.host_str(),
            Some(
                "youtube.com"
                    | "www.youtube.com"
                    | "m.youtube.com"
                    | "music.youtube.com"
                    | "youtu.be"
                    | "www.youtu.be"
            )
        )
}

pub fn video_id(input: &str) -> Option<String> {
    let url = Url::parse(input).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let id = match host.as_str() {
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com" => {
            match url.path() {
                "/watch" => url
                    .query_pairs()
                    .find(|(key, _)| key == "v")?
                    .1
                    .into_owned(),
                path if path.starts_with("/shorts/")
                    || path.starts_with("/live/")
                    || path.starts_with("/embed/") =>
                {
                    path.split('/').nth(2)?.to_owned()
                }
                _ => return None,
            }
        }
        "youtu.be" | "www.youtu.be" => url.path().trim_start_matches('/').to_owned(),
        _ => return None,
    };
    (id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
    .then_some(id)
}

fn choose_tracks(
    formats: &[StreamingFormat],
    max_height: u32,
) -> io::Result<(&StreamingFormat, &StreamingFormat)> {
    let video = formats
        .iter()
        .filter(|f| {
            f.is_video_only()
                && f.mime_type.starts_with("video/mp4; codecs=\"avc1.")
                && f.height.is_some_and(|h| h <= max_height)
                && f.index_range.is_some()
                && f.init_range.is_some()
                && f.drm_families.as_ref().is_none_or(Vec::is_empty)
        })
        .max_by_key(|f| (f.height.unwrap_or(0), f.bitrate))
        .ok_or_else(|| io::Error::other("YouTube has no indexed H.264 MP4 video track"))?;
    let audio = formats
        .iter()
        .filter(|f| {
            f.is_audio_only()
                && f.mime_type.starts_with("audio/mp4; codecs=\"mp4a.")
                && f.index_range.is_some()
                && f.init_range.is_some()
                && f.drm_families.as_ref().is_none_or(Vec::is_empty)
        })
        .max_by_key(|f| f.bitrate)
        .ok_or_else(|| io::Error::other("YouTube has no indexed AAC MP4 audio track"))?;
    Ok((video, audio))
}

pub async fn resolve(input: &str, max_height: u32) -> io::Result<YoutubeDash> {
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        resolve_inner(input, max_height),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "YouTube extraction timeout"))?
}

async fn resolve_inner(input: &str, max_height: u32) -> io::Result<YoutubeDash> {
    let id = video_id(input).ok_or_else(|| io::Error::other("invalid YouTube video URL"))?;
    let yt = YOUTUBE_CLIENT
        .get_or_try_init(|| async { Innertube::new().await })
        .await
        .map_err(io::Error::other)?;
    // The high-level get_video_info silently falls back to IOS, whose CDN
    // URLs can read the beginning but return 403 for later ranges without PO
    // attestation. Request VISIONOS explicitly and preserve its failure rather
    // than returning a different client's unusable tracks.
    let response = yt
        .session
        .post_innertube_client(
            "VISIONOS",
            "/player",
            serde_json::json!({
                "videoId": id,
                "contentCheckOk": true,
                "racyCheckOk": true
            }),
        )
        .await
        .map_err(io::Error::other)?
        .json::<innertube_rs::PlayerResponse>()
        .await
        .map_err(|error| io::Error::other(error.without_url()))?;
    if response.playability_status.status != "OK" {
        return Err(io::Error::other(format!(
            "YouTube video is not playable: {} ({})",
            response.playability_status.status,
            response
                .playability_status
                .reason
                .as_deref()
                .unwrap_or("no reason provided")
        )));
    }
    let data = response.streaming_data.as_ref().ok_or_else(|| {
        io::Error::other(format!(
            "YouTube video is not streamable: {}",
            response.playability_status.status
        ))
    })?;
    let (video, audio) = choose_tracks(&data.adaptive_formats, max_height)?;
    let resolve = |format| {
        resolve_stream_url_full(
            format,
            &yt.player.decipherer,
            yt.session.po_token.as_deref(),
            None,
        )
        .map_err(io::Error::other)
    };
    let video_url = resolve(video)?;
    let audio_url = resolve(audio)?;
    let duration_secs = response
        .video_details
        .as_ref()
        .and_then(|details| details.length_seconds.parse().ok())
        .unwrap_or(0);
    Ok(YoutubeDash {
        video_url,
        audio_url,
        duration_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::{choose_tracks, video_id};
    use innertube_rs::StreamingFormat;

    fn indexed_format(itag: u32, mime: &str, height: Option<u32>) -> StreamingFormat {
        let mut format = StreamingFormat::default();
        format.itag = itag;
        format.mime_type = mime.into();
        format.height = height;
        format.bitrate = 1000;
        format.init_range =
            Some(serde_json::from_value(serde_json::json!({"start":"0","end":"99"})).unwrap());
        format.index_range =
            Some(serde_json::from_value(serde_json::json!({"start":"100","end":"199"})).unwrap());
        format
    }

    #[test]
    fn chooses_compatible_tracks_within_the_quality_limit() {
        let formats = vec![
            indexed_format(136, "video/mp4; codecs=\"avc1.4d401f\"", Some(720)),
            indexed_format(137, "video/mp4; codecs=\"avc1.640028\"", Some(1080)),
            indexed_format(400, "video/mp4; codecs=\"av01.0.08M.08\"", Some(1080)),
            indexed_format(140, "audio/mp4; codecs=\"mp4a.40.2\"", None),
            indexed_format(251, "audio/webm; codecs=\"opus\"", None),
        ];
        for (limit, expected) in [(720, 136), (1080, 137), (2160, 137)] {
            let (video, audio) = choose_tracks(&formats, limit).unwrap();
            assert_eq!((video.itag, audio.itag), (expected, 140));
        }
        assert!(choose_tracks(&formats, 360).is_err());
    }

    #[test]
    fn rejects_missing_audio_indexes_and_drm_without_falling_back_to_a_page_url() {
        let video = indexed_format(137, "video/mp4; codecs=\"avc1.640028\"", Some(1080));
        let audio = indexed_format(140, "audio/mp4; codecs=\"mp4a.40.2\"", None);
        assert!(choose_tracks(std::slice::from_ref(&video), 1080).is_err());
        let mut no_index = video.clone();
        no_index.index_range = None;
        assert!(choose_tracks(&[no_index, audio.clone()], 1080).is_err());
        let mut drm = video.clone();
        drm.drm_families = Some(vec!["WIDEVINE".into()]);
        assert!(choose_tracks(&[drm, audio.clone()], 1080).is_err());
        let mut no_init = audio;
        no_init.init_range = None;
        assert!(choose_tracks(&[video, no_init], 1080).is_err());
    }

    #[test]
    fn accepts_only_youtube_video_pages() {
        assert_eq!(
            video_id("https://youtu.be/dQw4w9WgXcQ?t=30").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(
            video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=x").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert!(video_id("https://youtube.com.evil.test/watch?v=dQw4w9WgXcQ").is_none());
        assert!(super::is_youtube_url("https://youtube.com/playlist?list=x"));
        assert!(video_id("https://youtube.com/playlist?list=x").is_none());
    }

    /// Manual network smoke test: validates both the extractor and the existing
    /// indexed DASH path against the reported 241-second failure, without a
    /// DLNA renderer. Midpoint/tail reads must pass beyond the initial window.
    #[actix_web::test]
    #[ignore = "requires live YouTube access"]
    async fn public_video_can_prepare_for_dlna() {
        check_public_video(crate::cast::Quality::P720, 720).await;
    }

    #[actix_web::test]
    #[ignore = "requires live YouTube access"]
    async fn public_video_1080p_can_prepare_for_dlna() {
        check_public_video(crate::cast::Quality::P1080, 1080).await;
    }

    async fn check_public_video(quality: crate::cast::Quality, expected_height: u16) {
        use crate::{
            SharedState,
            media_server::proxy_handler,
            media_session::{MediaSessions, SessionMedia},
        };
        use actix_web::{
            App,
            http::{Method, StatusCode},
            test, web,
        };
        use std::{collections::HashMap, sync::Arc};

        let sessions = Arc::new(MediaSessions::default());
        let session = sessions.activate("https://www.youtube.com/watch?v=_1gTMhvRtB8", quality);
        let started = std::time::Instant::now();
        let client = reqwest::Client::new();
        let media = session
            .prepare(&client)
            .await
            .expect("prepare YouTube session");
        let SessionMedia::Seekable { mp4, .. } = media.as_ref() else {
            panic!("YouTube must be proxied as a seekable MP4");
        };
        assert!(mp4.len > mp4.prefix.len() as u64);
        // Inspect the actual muxed representation: a lower-resolution fallback
        // must not count as a successful test of the requested quality.
        let parsed = mp4::Mp4Reader::read_header(std::io::Cursor::new(&mp4.prefix), mp4.len)
            .expect("parse muxed MP4 header");
        assert_eq!(parsed.tracks().len(), 2);
        let video = parsed
            .tracks()
            .values()
            .find(|track| track.media_type().ok() == Some(mp4::MediaType::H264))
            .expect("muxed H.264 video track");
        assert_eq!(
            video.height(),
            expected_height,
            "unexpected quality fallback"
        );
        assert!(
            parsed
                .tracks()
                .values()
                .any(|track| track.media_type().ok() == Some(mp4::MediaType::AAC))
        );
        eprintln!(
            "Muxed output: {}x{}, H.264 + AAC, duration={:?}, bytes={}, prepare={:?}",
            video.width(),
            video.height(),
            parsed.duration(),
            mp4.len,
            started.elapsed()
        );
        let first_media_byte = mp4.prefix.len();
        let state = web::Data::new(SharedState {
            duration_cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            eplus_auth: Arc::new(tokio::sync::Mutex::new(None)),
            media_sessions: sessions,
        });
        let app = test::init_service(
            App::new()
                .app_data(state)
                .app_data(web::Data::new(client))
                .service(proxy_handler),
        )
        .await;
        let path = format!("/{}", session.path());
        let head = test::TestRequest::with_uri(&path)
            .method(Method::HEAD)
            .to_request();
        let head_response = test::call_service(&app, head).await;
        assert_eq!(head_response.status(), StatusCode::OK);
        assert_eq!(
            head_response.headers().get("content-type").unwrap(),
            "video/mp4"
        );
        for start in [first_media_byte as u64, mp4.len / 2, mp4.len - 65536] {
            let get = test::TestRequest::with_uri(&path)
                .insert_header(("Range", format!("bytes={start}-{}", start + 65535)))
                .to_request();
            let response = test::call_service(&app, get).await;
            assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
            assert_eq!(
                response
                    .headers()
                    .get("content-range")
                    .unwrap()
                    .to_str()
                    .unwrap(),
                format!("bytes {start}-{}/{}", start + 65535, mp4.len)
            );
            assert_eq!(test::read_body(response).await.len(), 65536);
            eprintln!(
                "Proxy Range passed: {start}-{} (65536 bytes)",
                start + 65535
            );
        }
    }
}
