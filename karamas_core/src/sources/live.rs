//! 从页面播放配置解析直播源。
use crate::cast::Quality;
use futures_util::StreamExt;
use serde_json::Value;
use std::{io, time::Duration};
use url::Url;

// 按 KID 查找 AES-128 密钥；音视频可以使用不同密钥，仅在会话内存中保留。
pub(crate) use crate::media::mp4::cenc::Keys;

#[derive(Clone)]
pub(crate) enum Source {
    Hls(Url),
    Dash { manifest: Url, keys: Keys },
}

#[derive(Clone)]
pub(crate) struct Resolved {
    pub source: Source,
    pub referer: String,
}

pub(crate) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(crate) fn media_url(base: &Url, value: &str) -> io::Result<Url> {
    let url = base.join(value).map_err(|_| invalid("直播媒体地址无效"))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid("直播媒体地址必须使用 HTTP 或 HTTPS"));
    }
    Ok(url)
}

/// 限制页面、清单和单个分片的大小；错误信息不携带上游查询参数。
pub(crate) async fn fetch(
    client: &reqwest::Client,
    url: &Url,
    referer: &str,
    limit: usize,
) -> io::Result<(Url, bytes::Bytes)> {
    let response = client
        .get(url.clone())
        .header("User-Agent", "Mozilla/5.0")
        .header("Referer", referer)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| io::Error::other(error.without_url()))?;
    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "读取直播媒体失败：HTTP {}",
            response.status()
        )));
    }
    let effective = response.url().clone();
    Ok((effective, read_body(response, limit).await?))
}

async fn read_body(response: reqwest::Response, limit: usize) -> io::Result<bytes::Bytes> {
    let mut stream = response.bytes_stream();
    let mut data = bytes::BytesMut::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| io::Error::other(error.without_url()))?;
        if data.len().saturating_add(chunk.len()) > limit {
            return Err(invalid("直播媒体响应超过大小限制"));
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data.freeze())
}

/// 明确标明格式的媒体直链不探测网页；其余 HTTP 地址按响应类型判断。
pub(crate) async fn resolve(client: &reqwest::Client, value: &str) -> io::Result<Option<Resolved>> {
    let Ok(page) = Url::parse(value) else {
        return Ok(None);
    };
    if !matches!(page.scheme(), "http" | "https")
        || crate::sources::youtube::is_youtube_url(value)
        || crate::sources::bilibili::parse_bilibili_video(value).is_ok()
    {
        return Ok(None);
    }
    let extension = page
        .path()
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        extension.as_str(),
        "mp4"
            | "m4v"
            | "mkv"
            | "webm"
            | "mov"
            | "flv"
            | "mp3"
            | "m4a"
            | "aac"
            | "ogg"
            | "wav"
            | "m3u8"
            | "mpd"
            | "ts"
            | "m4s"
    ) {
        return Ok(None);
    }
    let response = client
        .get(page.clone())
        .header("User-Agent", "Mozilla/5.0")
        .header(
            "Referer",
            format!("{}/", page.origin().ascii_serialization()),
        )
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| io::Error::other(error.without_url()))?;
    let html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            matches!(
                value
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase()
                    .as_str(),
                "text/html" | "application/xhtml+xml"
            )
        });
    if !html {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "读取直播页面失败：HTTP {}",
            response.status()
        )));
    }
    // 配置中的相对地址以最终页面为基准，支持换域名和重定向后的房间路径。
    let page = response.url().clone();
    let body = read_body(response, 4 * 1024 * 1024).await?;
    let html = std::str::from_utf8(&body).map_err(|_| invalid("直播页面不是 UTF-8"))?;
    let config = page_config(html).ok_or_else(|| invalid("网页没有支持的直播播放配置"))?;
    let source = source(&page, &config)?;
    Ok(Some(Resolved {
        source,
        referer: format!("{}/", page.origin().ascii_serialization()),
    }))
}

fn source(page: &Url, config: &Value) -> io::Result<Source> {
    if config.get("urls").is_some() {
        if config["isLive"].as_bool() != Some(true) {
            return Err(invalid("当前页面没有提供直播"));
        }
        let urls = &config["urls"];
        let address = if let Some(address) = urls.as_str() {
            address
        } else {
            let variants = urls
                .as_object()
                .ok_or_else(|| invalid("页面媒体地址格式不支持"))?;
            let mut selected = None;
            for (label, value) in variants {
                let height = label
                    .trim_end_matches(['p', 'P'])
                    .parse::<u32>()
                    .ok()
                    .filter(|height| *height > 0)
                    .ok_or_else(|| invalid("播放配置含有不支持的画质标签"))?;
                let address = value
                    .as_str()
                    .ok_or_else(|| invalid("画质表中的媒体地址不是字符串"))?;
                if selected.is_none_or(|(current, _)| height > current) {
                    selected = Some((height, address));
                }
            }
            selected.ok_or_else(|| invalid("页面没有可用的媒体地址"))?.1
        };
        let url = media_url(page, address)?;
        return Ok(Source::Hls(url));
    }
    let dash = match config["sourceType"].as_str() {
        Some("dash") => true,
        Some("hls") => false,
        _ => return Err(invalid("不支持此房间的直播源类型")),
    };
    // 页面播放器通过 /room/{slug}/index.mpd 或 index.m3u8 访问媒体。
    let slug = config["slug"]
        .as_str()
        .filter(|slug| !slug.is_empty())
        .ok_or_else(|| invalid("播放配置缺少房间标识，无法确定媒体入口"))?;
    let mut manifest = page.clone();
    manifest
        .path_segments_mut()
        .map_err(|_| invalid("直播媒体入口地址无效"))?
        .clear()
        .extend(["room", slug, if dash { "index.mpd" } else { "index.m3u8" }]);
    manifest.set_query(None);
    manifest.set_fragment(None);
    if !dash {
        // 页面播放器在非空 keyUrl 时启用自定义 HLS 解密；当前未接入这套扩展。
        if config["keyUrl"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
        {
            return Err(invalid("当前不支持页面播放器的自定义 HLS 解密"));
        }
        return Ok(Source::Hls(manifest));
    }
    let keys = config["keys"]
        .as_object()
        .map(|object| {
            object
                .iter()
                .map(|(kid, key)| {
                    Ok((
                        hex_key(kid)?,
                        hex_key(key.as_str().ok_or_else(|| invalid("直播密钥格式无效"))?)?,
                    ))
                })
                .collect::<io::Result<Keys>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(Source::Dash { manifest, keys })
}

fn page_config(html: &str) -> Option<Value> {
    let mut payload = String::new();
    for part in html.split("self.__next_f.push(").skip(1) {
        // 从 JSON 本身确定字符串边界，避免依赖 Next.js 的 chunk 名称和模块编号。
        let mut values = serde_json::Deserializer::from_str(part).into_iter::<Value>();
        if let Some(Ok(value)) = values.next()
            && value[0].as_u64() == Some(1)
            && let Some(text) = value[1].as_str()
        {
            payload.push_str(text);
        }
    }
    fn find(value: &Value) -> Option<&Value> {
        if value.is_object() && (value.get("urls").is_some() || value.get("sourceType").is_some()) {
            return Some(value);
        }
        match value {
            Value::Array(values) => values.iter().find_map(find),
            Value::Object(values) => values.values().find_map(find),
            _ => None,
        }
    }
    // RSC 的每行是 id:JSON 记录；只解析 JSON 内容，跳过模块引用等其他记录。
    payload
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.split_once(':')?.1).ok())
        .find_map(|value| find(&value).cloned())
}

pub(crate) fn hex_key(text: &str) -> io::Result<[u8; 16]> {
    let text = text.replace('-', "");
    if text.len() != 32 || !text.is_ascii() {
        return Err(invalid("直播密钥格式无效"));
    }
    let mut key = [0; 16];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| invalid("直播密钥格式无效"))?;
    }
    Ok(key)
}

pub(crate) fn target_height(quality: Quality) -> u32 {
    match quality {
        Quality::P240 => 240,
        Quality::P360 => 360,
        Quality::P480 => 480,
        Quality::P720 | Quality::P720P60 => 720,
        Quality::P4K | Quality::Hdr | Quality::Dolby => 2160,
        Quality::P8K => 4320,
        _ => 1080,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 同样的播放配置应适用于任意域名和路径，源地址按页面 URL 解析。
    #[test]
    fn page_configuration_resolves_without_a_site_registry() {
        let config = serde_json::json!({"isLive":true,"urls":"../stream/index.m3u8?sig=test"});
        for page in [
            "https://unregistered.example/watch/channel",
            "http://localhost:18080/custom/room",
        ] {
            let page = Url::parse(page).unwrap();
            let Source::Hls(url) = source(&page, &config).unwrap() else {
                panic!("expected HLS")
            };
            assert_eq!(url, page.join("../stream/index.m3u8?sig=test").unwrap());
            let config = serde_json::json!({"slug":"channel","sourceType":"dash","keys":{}});
            let Source::Dash { manifest, .. } = source(&page, &config).unwrap() else {
                panic!("expected DASH")
            };
            assert_eq!(manifest, page.join("/room/channel/index.mpd").unwrap());
        }
        assert!(
            source(
                &Url::parse("https://example.test/room").unwrap(),
                &serde_json::json!({"isLive":false,"urls":"live.m3u8"})
            )
            .is_err()
        );
    }

    // 页面配置可能跨多个 RSC 字符串传输，解析不应依赖模块号或脚本文件名。
    #[test]
    fn next_page_configuration_can_span_transport_chunks() {
        let config = "5:[\"$\",\"$Lc\",null,{\"urls\":\"https://cdn.example/live.m3u8?a=1&b=2\",\"isLive\":true}]\n";
        let (first, second) = config.split_at(35);
        let html = format!(
            "<script>self.__next_f.push({})</script><script>self.__next_f.push({})</script>",
            serde_json::json!([1, first]),
            serde_json::json!([1, second])
        );
        assert_eq!(
            page_config(&html).unwrap()["urls"],
            "https://cdn.example/live.m3u8?a=1&b=2"
        );
    }
}
