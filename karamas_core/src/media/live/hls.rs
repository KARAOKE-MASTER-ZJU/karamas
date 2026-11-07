//! 重写 HLS 中的相对地址，所有清单、分片和密钥都留在当前 token 会话下。
use super::Resource;
use crate::sources::live::{fetch, invalid, media_url};
use std::{collections::VecDeque, io, sync::Mutex};
use url::Url;

struct Entry {
    id: u64,
    url: Url,
    playlist: bool,
}

struct Resources {
    entries: VecDeque<Entry>,
    next_id: u64,
}

pub(super) struct Hls {
    root: Url,
    resources: Mutex<Resources>,
}

impl Hls {
    pub async fn new(client: &reqwest::Client, root: Url, referer: &str) -> io::Result<Self> {
        let (_, body) = fetch(client, &root, referer, 2 * 1024 * 1024).await?;
        if !body.starts_with(b"#EXTM3U") {
            return Err(invalid("直播源没有返回 HLS 清单"));
        }
        Ok(Self {
            root,
            resources: Mutex::new(Resources {
                entries: VecDeque::new(),
                next_id: 1,
            }),
        })
    }

    pub async fn resource(
        &self,
        client: &reqwest::Client,
        name: &str,
        referer: &str,
    ) -> io::Result<Resource> {
        let url = if name == "master.m3u8" {
            self.root.clone()
        } else {
            let id: u64 = name
                .strip_prefix("resource-")
                .and_then(|name| name.split_once('.'))
                .and_then(|(id, _)| id.parse().ok())
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            let resources = self.resources.lock().unwrap();
            let entry = resources
                .entries
                .iter()
                .find(|entry| entry.id == id)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "直播资源已过期"))?;
            if !entry.playlist {
                return Ok(Resource::Remote(entry.url.clone()));
            }
            entry.url.clone()
        };
        // 相对 URI 以重定向后的清单地址为基准，避免 CDN 重定向后目录解析错误。
        let (effective, body) = fetch(client, &url, referer, 2 * 1024 * 1024).await?;
        let text = std::str::from_utf8(&body).map_err(|_| invalid("HLS 清单不是 UTF-8"))?;
        if !text.trim_start_matches('\u{feff}').starts_with("#EXTM3U") {
            return Err(invalid("直播源没有返回 HLS 清单"));
        }
        let mut resources = self.resources.lock().unwrap();
        Ok(Resource::Playlist(rewrite(
            text,
            &effective,
            &mut resources,
        )?))
    }
}

fn rewrite(text: &str, base: &Url, resources: &mut Resources) -> io::Result<String> {
    let mut output = String::new();
    let mut variant = false;
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !line.starts_with('#') {
            output.push_str(&register(resources, media_url(base, line)?, variant)?);
            variant = false;
        } else {
            // RFC 8216 的标签决定 URI 类型，子清单地址可以没有 .m3u8 扩展名。
            let tag = line.split(':').next().unwrap_or_default();
            let playlist = matches!(
                tag,
                "#EXT-X-MEDIA" | "#EXT-X-I-FRAME-STREAM-INF" | "#EXT-X-RENDITION-REPORT"
            );
            if tag == "#EXT-X-STREAM-INF" {
                variant = true;
            }
            let mut rest = line;
            while let Some((prefix, suffix)) = rest.split_once("URI=\"") {
                let (value, tail) = suffix
                    .split_once('"')
                    .ok_or_else(|| invalid("HLS 资源地址引号不完整"))?;
                output.push_str(prefix);
                output.push_str("URI=\"");
                output.push_str(&register(resources, media_url(base, value)?, playlist)?);
                output.push('"');
                rest = tail;
            }
            output.push_str(rest);
        }
        output.push('\n');
    }
    Ok(output)
}

fn register(resources: &mut Resources, url: Url, playlist: bool) -> io::Result<String> {
    // 部分播放端按扩展名校验 HLS 分片，代理地址须保留实际媒体类型。
    let extension = url
        .path()
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let extension = if playlist {
        "m3u8"
    } else {
        match extension.as_str() {
            "m3u8" | "ts" | "m4s" | "mp4" | "aac" | "ac3" | "ec3" | "vtt" | "webvtt" | "key" => {
                extension.as_str()
            }
            _ => "bin",
        }
    };
    let id = if let Some(entry) = resources
        .entries
        .iter()
        .find(|entry| entry.url == url && entry.playlist == playlist)
    {
        entry.id
    } else {
        let id = resources.next_id;
        resources.next_id = id
            .checked_add(1)
            .ok_or_else(|| invalid("直播资源编号耗尽"))?;
        if resources.entries.len() >= 1024 {
            resources.entries.pop_front();
        }
        resources.entries.push_back(Entry { id, url, playlist });
        id
    };
    Ok(format!("resource-{id}.{}", extension))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 播放端必须只请求当前会话，清单相对地址、属性 URI 和上游签名都不能丢失。
    #[test]
    fn playlists_keep_signed_resources_inside_the_current_session() {
        let base = Url::parse("https://cdn.example/live/channel/index.m3u8?auth=manifest").unwrap();
        let text = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"../key?sig=key\"\n#EXT-X-MAP:URI=\"init.mp4?sig=init\"\n#EXTINF:4,\npart.ts?sig=segment\n#EXT-X-STREAM-INF:BANDWIDTH=1000\n../variant/playlist?sig=variant\n";
        let mut resources = Resources {
            entries: VecDeque::new(),
            next_id: 1,
        };
        let output = rewrite(text, &base, &mut resources).unwrap();
        assert!(!output.contains("https://"));
        assert!(!output.contains("sig="));
        assert!(output.contains("URI=\"resource-1.bin\""));
        assert!(output.contains("resource-4.m3u8"));
        assert!(output.contains("resource-3.ts"));
        assert_eq!(
            resources.entries[0].url.as_str(),
            "https://cdn.example/live/key?sig=key"
        );
        assert_eq!(resources.entries[2].url.query(), Some("sig=segment"));
        assert_eq!(rewrite(text, &base, &mut resources).unwrap(), output);
        assert_eq!(resources.entries.len(), 4);
    }
}
