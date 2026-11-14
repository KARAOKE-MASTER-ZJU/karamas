//! 修改普通 MP4 的轨道时间轴，媒体数据按原文件范围读取。
use super::{
    VirtualMp4,
    layout::Extent,
    moov::{delay_track, duration, replace, time_scale},
};
use crate::media::{
    cache::RemoteSource,
    mp4::{
        boxes::{atom, child, children, header, invalid, u32_at, u64_at},
        dash::{PreparedDash, source_prefix},
    },
    upstream::fetch_range,
};
use bytes::Bytes;
use std::{io, ops::Range};

pub(crate) async fn prepare(
    client: &reqwest::Client,
    url: &str,
    delay_ms: i32,
) -> io::Result<PreparedDash> {
    let (total, prefix) = source_prefix(client, url).await?;
    let mut position = 0;
    let mut ftyp = None;
    let mut moov = None;
    let mut retained = Vec::new();
    let mut payloads = Vec::new();
    while position < total {
        if retained.len() >= 128 || total - position < 8 {
            return Err(invalid("MP4 顶层结构无效"));
        }
        let end = position.saturating_add(16).min(total);
        let head = read(client, url, position..end, total, &prefix).await?;
        let h = header(&head)? as u64;
        let size = match u32_at(&head, 0)? {
            0 => total - position,
            1 => u64_at(&head, 8)?,
            n => u64::from(n),
        };
        let end = position
            .checked_add(size)
            .filter(|end| size >= h && *end <= total)
            .ok_or_else(|| invalid("MP4 数据块超出文件范围"))?;
        match &head[4..8] {
            b"ftyp" | b"moov" => {
                if size > 16 * 1024 * 1024 {
                    return Err(invalid("MP4 元数据超过内存上限"));
                }
                let data = read(client, url, position..end, total, &prefix).await?;
                let slot = if &head[4..8] == b"ftyp" {
                    &mut ftyp
                } else {
                    &mut moov
                };
                if slot.replace(data).is_some() {
                    return Err(invalid("MP4 元数据重复"));
                }
            }
            b"moof" | b"sidx" | b"mfra" => {
                return Err(invalid("分片 MP4 需要通过分段索引处理"));
            }
            kind => {
                retained.push(position..end);
                if kind == b"mdat" {
                    payloads.push(position + h..end);
                }
            }
        }
        position = end;
    }
    let ftyp = ftyp.ok_or_else(|| invalid("MP4 缺少文件类型"))?;
    let moov = moov.ok_or_else(|| invalid("MP4 缺少影片元数据"))?;
    if payloads.is_empty() {
        return Err(invalid("MP4 缺少媒体数据"));
    }
    let shifted = shift_movie(&moov, delay_ms)?;
    // 先统一成 co64，第二遍写地址时 moov 长度不再变化。
    let sized = chunk_offsets(&shifted, &|offset| Ok(offset))?;
    let mut output = (ftyp.len() + sized.len()) as u64;
    let mut extents = Vec::new();
    for range in retained {
        let end = output
            .checked_add(range.end - range.start)
            .ok_or_else(|| invalid("MP4 文件长度溢出"))?;
        extents.push(Extent {
            output: output..end,
            source: 0,
            source_start: range.start,
        });
        output = end;
    }
    let moov = chunk_offsets(&shifted, &|offset| {
        if !payloads.iter().any(|range| range.contains(&offset)) {
            return Err(invalid("MP4 采样块超出媒体数据范围"));
        }
        let extent = extents
            .iter()
            .find(|e| {
                (e.source_start..e.source_start + (e.output.end - e.output.start)).contains(&offset)
            })
            .ok_or_else(|| invalid("MP4 缺少采样块映射"))?;
        Ok(extent.output.start + offset - extent.source_start)
    })?;
    if moov.len() != sized.len() {
        return Err(invalid("MP4 元数据长度已变化"));
    }
    let mut prefix = ftyp.to_vec();
    prefix.extend_from_slice(&moov);
    let mp4 = VirtualMp4 {
        prefix: Bytes::from(prefix),
        len: output,
        extents,
        seek_points: Vec::new(),
    };
    mp4.validate_sources(&[total])?;
    Ok(PreparedDash {
        mp4,
        sources: vec![RemoteSource {
            url: url.into(),
            len: total,
        }],
    })
}

async fn read(
    client: &reqwest::Client,
    url: &str,
    range: Range<u64>,
    total: u64,
    prefix: &Bytes,
) -> io::Result<Bytes> {
    if range.end <= prefix.len() as u64 {
        Ok(prefix.slice(range.start as usize..range.end as usize))
    } else {
        fetch_range(client, url, range, total).await
    }
}

fn header_duration(b: &[u8], track: bool) -> io::Result<u64> {
    let h = header(b)?;
    match b.get(h) {
        Some(0) => Ok(u64::from(u32_at(b, h + if track { 20 } else { 16 })?)),
        Some(1) => u64_at(b, h + if track { 28 } else { 24 }),
        _ => Err(invalid("MP4 时长版本不受支持")),
    }
}

fn shift_movie(moov: &[u8], delay_ms: i32) -> io::Result<Vec<u8>> {
    let mvhd = child(moov, b"mvhd")?;
    let scale = time_scale(mvhd)?;
    let delay = (u64::from(delay_ms.unsigned_abs()) * u64::from(scale)).div_ceil(1000);
    let mut movie_duration = header_duration(mvhd, false)?;
    let mut body = Vec::new();
    let mut found = false;
    for b in children(moov)? {
        if &b[4..8] == b"mvex" {
            return Err(invalid("分片 MP4 不能按普通 MP4 处理"));
        }
        if &b[4..8] == b"mvhd" {
            continue;
        }
        if &b[4..8] != b"trak" {
            body.extend_from_slice(b);
            continue;
        }
        let mdia = child(b, b"mdia")?;
        let handler = child(mdia, b"hdlr")?;
        let h = header(handler)?;
        let selected =
            handler.get(h + 8..h + 12) == Some(if delay_ms > 0 { b"soun" } else { b"vide" });
        if selected {
            found = true;
            let tkhd = child(b, b"tkhd")?;
            let original = header_duration(tkhd, true)?;
            let extended = original
                .checked_add(delay)
                .ok_or_else(|| invalid("MP4 时长溢出"))?;
            movie_duration = movie_duration.max(extended);
            let shifted = delay_track(b, delay, original)?;
            body.extend_from_slice(&replace(
                &shifted,
                b"tkhd",
                &duration(tkhd, extended, true)?,
            )?);
        } else {
            body.extend_from_slice(b);
        }
    }
    if !found {
        return Err(invalid("MP4 缺少需要调整的音视频轨道"));
    }
    let mut result = duration(mvhd, movie_duration, false)?;
    result.extend_from_slice(&body);
    atom(b"moov", &result)
}

fn chunk_offsets(b: &[u8], map: &impl Fn(u64) -> io::Result<u64>) -> io::Result<Vec<u8>> {
    match &b[4..8] {
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" => {
            let mut body = Vec::new();
            for c in children(b)? {
                body.extend_from_slice(&chunk_offsets(c, map)?);
            }
            atom(b[4..8].try_into().unwrap(), &body)
        }
        b"stco" | b"co64" => {
            let h = header(b)?;
            let width = if &b[4..8] == b"stco" { 4 } else { 8 };
            let count = u32_at(b, h + 4)? as usize;
            let remaining = b.len() - h - 8;
            if count != remaining / width || remaining % width != 0 {
                return Err(invalid("MP4 采样块索引长度无效"));
            }
            let mut body = b[h..h + 8].to_vec();
            for n in 0..count {
                let p = h + 8 + n * width;
                let offset = if width == 4 {
                    u64::from(u32_at(b, p)?)
                } else {
                    u64_at(b, p)?
                };
                body.extend_from_slice(&map(offset)?.to_be_bytes());
            }
            atom(b"co64", &body)
        }
        b"saio" | b"senc" => Err(invalid("加密 MP4 不支持调整轨道时间轴")),
        _ => Ok(b.to_vec()),
    }
}
