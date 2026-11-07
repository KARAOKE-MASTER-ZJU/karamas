//! 直播媒体会话：页面取源、HLS 资源代理和 DASH 分片解密。
mod dash;
mod hls;

use crate::{
    cast::Quality,
    sources::live::{Resolved, Source},
};
use std::io;
use url::Url;

pub(crate) enum Resource {
    Playlist(String),
    File(std::sync::Arc<tempfile::NamedTempFile>),
    Remote(Url),
}

pub struct LiveMedia {
    kind: Kind,
    referer: String,
    quality: Quality,
}

enum Kind {
    Hls(hls::Hls),
    Dash(Box<dash::Dash>),
}

impl LiveMedia {
    pub(crate) async fn prepare(
        client: &reqwest::Client,
        resolved: Resolved,
        quality: Quality,
    ) -> io::Result<Self> {
        let Resolved { source, referer } = resolved;
        let (kind, actual) = match source {
            Source::Hls(url) => (
                Kind::Hls(hls::Hls::new(client, url, &referer).await?),
                quality,
            ),
            Source::Dash { manifest, keys } => {
                let media = dash::Dash::new(client, manifest, &referer, keys, quality).await?;
                let actual = media.quality();
                (Kind::Dash(Box::new(media)), actual)
            }
        };
        Ok(Self {
            kind,
            referer,
            quality: actual,
        })
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    pub(crate) fn referer(&self) -> &str {
        &self.referer
    }

    pub(crate) async fn resource(
        &self,
        client: &reqwest::Client,
        name: &str,
    ) -> io::Result<Resource> {
        match &self.kind {
            Kind::Hls(media) => media.resource(client, name, &self.referer).await,
            Kind::Dash(media) => media.resource(client, name).await,
        }
    }
}

pub(crate) fn valid_resource(name: &str) -> bool {
    matches!(
        name,
        "master.m3u8" | "video.m3u8" | "audio.m3u8" | "video-init.mp4" | "audio-init.mp4"
    ) || ["video-", "audio-"].iter().any(|prefix| {
        name.strip_prefix(prefix)
            .and_then(|n| n.strip_suffix(".m4s"))
            .is_some_and(number)
    }) || name
        .strip_prefix("resource-")
        .and_then(|name| name.split_once('.'))
        .is_some_and(|(id, extension)| {
            number(id)
                && matches!(
                    extension,
                    "m3u8"
                        | "ts"
                        | "m4s"
                        | "mp4"
                        | "aac"
                        | "ac3"
                        | "ec3"
                        | "vtt"
                        | "webvtt"
                        | "key"
                        | "bin"
                )
        })
}

fn number(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok()
}
