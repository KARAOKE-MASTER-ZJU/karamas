//! Load only the metadata needed for a virtual ordinary MP4 from DASH sources.
//! Futures are structured (no detached tasks), so dropping preparation cancels
//! every outstanding upstream request when the current song changes.
use crate::{
    seekable_mp4::{Mp4Builder, VirtualMp4},
    song_cache::{RemoteSource, fetch_range},
};
use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt};
use std::{
    io,
    ops::Range,
    time::{Duration, Instant},
};

const PREFIX: u64 = 64 * 1024;
const FRAGMENT_PREFIX: u64 = 16 * 1024;
const MAX_METADATA: u64 = 16 * 1024 * 1024;
const MAX_SEGMENTS: usize = 8192;

fn bad(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn u32_at(b: &[u8], p: usize) -> io::Result<u32> {
    Ok(u32::from_be_bytes(
        b.get(p..p + 4)
            .ok_or_else(|| bad("truncated index"))?
            .try_into()
            .unwrap(),
    ))
}
fn u64_at(b: &[u8], p: usize) -> io::Result<u64> {
    Ok(u64::from_be_bytes(
        b.get(p..p + 8)
            .ok_or_else(|| bad("truncated index"))?
            .try_into()
            .unwrap(),
    ))
}
fn size(b: &[u8]) -> io::Result<(u64, usize)> {
    let (size, header) = match u32_at(b, 0)? {
        0 => return Err(bad("unbounded metadata box")),
        1 => (u64_at(b, 8)?, 16),
        n => (n as u64, 8),
    };
    if size < header as u64 {
        return Err(bad("invalid metadata box size"));
    }
    Ok((size, header))
}

fn segments(sidx: &[u8], position: u64, total: u64) -> io::Result<Vec<Range<u64>>> {
    let (length, h) = size(sidx)?;
    if length != sidx.len() as u64 || sidx.get(4..8) != Some(b"sidx") {
        return Err(bad("invalid sidx box"));
    }
    let version = *sidx.get(h).ok_or_else(|| bad("short sidx"))?;
    if version > 1 || u32_at(sidx, h + 8)? == 0 {
        return Err(bad("unsupported sidx version or timescale"));
    }
    let (first_offset, mut p) = if version == 0 {
        (u32_at(sidx, h + 16)? as u64, h + 20)
    } else {
        (u64_at(sidx, h + 20)?, h + 28)
    };
    let count = u32_at(sidx, p)? as u16 as usize; // reserved u16 followed by count u16
    p += 4;
    if count == 0 || count > MAX_SEGMENTS {
        return Err(bad("unsupported number of indexed segments"));
    }
    let mut offset = position
        .checked_add(length)
        .and_then(|n| n.checked_add(first_offset))
        .ok_or_else(|| bad("segment offset overflow"))?;
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        let reference = u32_at(sidx, p)?;
        if reference >> 31 != 0 {
            return Err(bad("nested segment indexes unsupported"));
        }
        let n = reference as u64;
        if n < 8 || u32_at(sidx, p + 4)? == 0 {
            return Err(bad("empty indexed segment"));
        }
        u32_at(sidx, p + 8)?; // require a complete SAP field even though seeking uses sample flags
        let end = offset
            .checked_add(n)
            .ok_or_else(|| bad("segment size overflow"))?;
        if end > total {
            return Err(bad("segment index exceeds source size"));
        }
        result.push(offset..end);
        offset = end;
        p += 12;
    }
    if p != sidx.len() {
        return Err(bad("unexpected sidx trailing data"));
    }
    Ok(result)
}

async fn source_length(client: &reqwest::Client, url: &str) -> io::Result<u64> {
    let response = client
        .get(url)
        .header("Range", "bytes=0-7")
        .header("Referer", crate::upstream::referer(url))
        .header("User-Agent", crate::upstream::user_agent(url))
        .header("Accept-Encoding", "identity")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| io::Error::other(e.without_url()))?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(bad("source does not support byte ranges"));
    }
    let range = response
        .headers()
        .get("Content-Range")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| bad("missing source range"))?;
    let total = range
        .strip_prefix("bytes 0-7/")
        .and_then(|n| n.parse::<u64>().ok())
        .ok_or_else(|| bad("invalid source range"))?;
    if total < 8 {
        return Err(bad("source is too short"));
    }
    Ok(total)
}

struct SourceMetadata {
    ftyp: Bytes,
    moov: Bytes,
    len: u64,
    fragments: Vec<(u64, Bytes, Range<u64>)>,
}

async fn load_source(client: &reqwest::Client, url: &str) -> io::Result<SourceMetadata> {
    let len = source_length(client, url).await?;
    let prefix = fetch_range(client, url, 0..len.min(PREFIX), len).await?;
    let mut position = 0u64;
    let mut ftyp = None;
    let mut moov = None;
    let mut index = None;
    for _ in 0..32 {
        if position.checked_add(16).is_none_or(|n| n > len) {
            return Err(bad("missing DASH initialization"));
        }
        let head = if position + 16 <= prefix.len() as u64 {
            prefix.slice(position as usize..position as usize + 16)
        } else {
            fetch_range(client, url, position..position + 16, len).await?
        };
        let (length, _) = size(&head)?;
        let end = position
            .checked_add(length)
            .ok_or_else(|| bad("metadata offset overflow"))?;
        if end > len {
            return Err(bad("metadata exceeds source"));
        }
        match &head[4..8] {
            b"ftyp" | b"moov" | b"sidx" => {
                if length > MAX_METADATA {
                    return Err(bad("metadata exceeds memory budget"));
                }
                let data = if end <= prefix.len() as u64 {
                    prefix.slice(position as usize..end as usize)
                } else {
                    fetch_range(client, url, position..end, len).await?
                };
                match &head[4..8] {
                    b"ftyp" => ftyp = Some(data),
                    b"moov" => moov = Some(data),
                    _ => {
                        index = Some(segments(&data, position, len)?);
                        break;
                    }
                }
            }
            b"moof" | b"mdat" => return Err(bad("DASH has no preceding segment index")),
            _ => {}
        }
        position = end;
    }
    let ftyp = ftyp.ok_or_else(|| bad("missing file type"))?;
    let moov = moov.ok_or_else(|| bad("missing movie metadata"))?;
    let index = index.ok_or_else(|| bad("missing segment index"))?;
    // Preserve source order despite concurrent requests: decode timestamps are
    // checked for continuity by Mp4Builder. At most four requests per source.
    let mut loads = futures_util::stream::iter(index.into_iter().map(|range| async move {
        let mut position = range.start;
        let prefix_end = range.end.min(position + FRAGMENT_PREFIX);
        let prefix = fetch_range(client, url, position..prefix_end, len).await?;
        for _ in 0..16 {
            if position.checked_add(16).is_none_or(|n| n > range.end) {
                return Err(bad("missing fragment header"));
            }
            let local = position - range.start;
            let head = if local + 16 <= prefix.len() as u64 {
                prefix.slice(local as usize..local as usize + 16)
            } else {
                fetch_range(client, url, position..position + 16, len).await?
            };
            let (length, _) = size(&head)?;
            let end = position
                .checked_add(length)
                .ok_or_else(|| bad("fragment offset overflow"))?;
            if end > range.end {
                return Err(bad("fragment exceeds indexed segment"));
            }
            if &head[4..8] == b"moof" {
                if length > MAX_METADATA {
                    return Err(bad("fragment metadata exceeds memory budget"));
                }
                let data = if end - range.start <= prefix.len() as u64 {
                    prefix.slice(local as usize..(end - range.start) as usize)
                } else {
                    fetch_range(client, url, position..end, len).await?
                };
                if end.checked_add(16).is_none_or(|n| n > range.end) {
                    return Err(bad("missing media payload after fragment"));
                }
                let media_local = end - range.start;
                let media_header = if media_local + 16 <= prefix.len() as u64 {
                    prefix.slice(media_local as usize..media_local as usize + 16)
                } else {
                    fetch_range(client, url, end..end + 16, len).await?
                };
                let (media_size, media_header_size) = size(&media_header)?;
                if &media_header[4..8] != b"mdat" || end.checked_add(media_size) != Some(range.end)
                {
                    return Err(bad(
                        "indexed segment must contain exactly one moof/mdat pair",
                    ));
                }
                return Ok((position, data, end + media_header_size as u64..range.end));
            }
            if &head[4..8] == b"mdat" {
                return Err(bad("media precedes fragment metadata"));
            }
            position = end;
        }
        Err(bad("too many boxes before fragment"))
    }))
    .buffered(4);
    let mut fragments = Vec::new();
    let mut metadata_bytes = 0;
    while let Some(fragment) = loads.try_next().await? {
        metadata_bytes += fragment.1.len() as u64;
        if metadata_bytes > 64 * 1024 * 1024 {
            return Err(bad("track metadata exceeds total budget"));
        }
        fragments.push(fragment);
    }
    Ok(SourceMetadata {
        ftyp,
        moov,
        len,
        fragments,
    })
}

pub struct PreparedDash {
    pub mp4: VirtualMp4,
    pub sources: Vec<RemoteSource>,
}

/// Preparation is cancellable by dropping this future. Call once per media
/// representation, before sharing it between HEAD/GET/Range requests.
pub async fn prepare(
    client: &reqwest::Client,
    video_url: &str,
    audio_url: &str,
) -> io::Result<PreparedDash> {
    let started = Instant::now();
    let (video, audio) = tokio::try_join!(
        load_source(client, video_url),
        load_source(client, audio_url)
    )?;
    let fragment_count = video.fragments.len() + audio.fragments.len();
    let mut builder = Mp4Builder::new(&video.ftyp, &video.moov, 0)?;
    builder.add_source(&audio.moov, 1)?;
    for (offset, moof, media) in video.fragments {
        builder.add_fragment_in_media(0, offset, &moof, media)?;
    }
    for (offset, moof, media) in audio.fragments {
        builder.add_fragment_in_media(1, offset, &moof, media)?;
    }
    let mp4 = builder.finish()?;
    mp4.validate_sources(&[video.len, audio.len])?;
    log::info!(target: "DLNA1080", "可定位 MP4 索引准备完成: elapsed_ms={}, fragments={}, header_bytes={}, file_bytes={}", started.elapsed().as_millis(), fragment_count, mp4.prefix.len(), mp4.len);
    Ok(PreparedDash {
        mp4,
        sources: vec![
            RemoteSource {
                url: video_url.to_owned(),
                len: video.len,
            },
            RemoteSource {
                url: audio_url.to_owned(),
                len: audio.len,
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn index(version: u8, sizes: &[u32]) -> Vec<u8> {
        let mut body = vec![version, 0, 0, 0];
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&1000u32.to_be_bytes());
        body.extend_from_slice(&vec![0; if version == 0 { 8 } else { 16 }]);
        body.extend_from_slice(&(sizes.len() as u32).to_be_bytes());
        for size in sizes {
            body.extend_from_slice(&size.to_be_bytes());
            body.extend_from_slice(&5000u32.to_be_bytes());
            body.extend_from_slice(&0x90000000u32.to_be_bytes());
        }
        let mut result = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        result.extend_from_slice(b"sidx");
        result.extend_from_slice(&body);
        result
    }
    #[test]
    fn segment_offsets_include_index_location_and_first_offset() {
        for version in [0, 1] {
            let mut idx = index(version, &[100, 200]);
            let offset_field = if version == 0 { 24 } else { 28 };
            if version == 0 {
                idx[offset_field..offset_field + 4].copy_from_slice(&20u32.to_be_bytes());
            } else {
                idx[offset_field..offset_field + 8].copy_from_slice(&20u64.to_be_bytes());
            }
            let first = 100 + idx.len() as u64 + 20;
            assert_eq!(
                segments(&idx, 100, first + 300).unwrap(),
                vec![first..first + 100, first + 100..first + 300]
            );
            assert!(segments(&idx, 100, first + 299).is_err());
        }
    }
    #[test]
    fn rejects_nested_and_truncated_indexes() {
        assert!(segments(&index(0, &[0x80000064]), 0, 10000).is_err());
        let idx = index(0, &[100]);
        for end in 0..idx.len() {
            assert!(segments(&idx[..end], 0, 10000).is_err());
        }
    }
}
