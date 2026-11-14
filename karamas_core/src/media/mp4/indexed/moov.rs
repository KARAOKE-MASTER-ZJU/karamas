//! 普通 MP4 的采样表、时长和 edit list 构建。
use super::Mp4Builder;
use crate::media::mp4::boxes::{atom, child, children, header, invalid, u32_at, u64_at};
use std::io;

/// 替换指定类型的直接子 box，并重建容器。
pub(super) fn replace(b: &[u8], kind: &[u8; 4], replacement: &[u8]) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    for c in children(b)? {
        body.extend_from_slice(if &c[4..8] == kind { replacement } else { c });
    }
    atom(b[4..8].try_into().unwrap(), &body)
}
/// 读取影片或媒体头中的时间刻度。
pub(super) fn time_scale(b: &[u8]) -> io::Result<u32> {
    let h = header(b)?;
    let version = *b.get(h).ok_or_else(|| invalid("short media header"))?;
    if version > 1 {
        return Err(invalid("unknown media header version"));
    }
    let value = u32_at(b, h + if version == 1 { 20 } else { 12 })?;
    if value == 0 {
        return Err(invalid("zero timescale"));
    }
    Ok(value)
}
/// 复制头部并更新影片、媒体或轨道时长。
pub(super) fn duration(b: &[u8], value: u64, track: bool) -> io::Result<Vec<u8>> {
    let mut out = b.to_vec();
    let h = header(b)?;
    let version = *b.get(h).ok_or_else(|| invalid("short duration box"))?;
    let p = h + match (version, track) {
        (0, false) => 16,
        (1, false) => 24,
        (0, true) => 20,
        (1, true) => 28,
        _ => return Err(invalid("unknown duration version")),
    };
    let bytes = if version == 1 {
        value.to_be_bytes().to_vec()
    } else {
        u32::try_from(value)
            .map_err(|_| invalid("duration exceeds v0 capacity"))?
            .to_be_bytes()
            .to_vec()
    };
    out.get_mut(p..p + bytes.len())
        .ok_or_else(|| invalid("short duration"))?
        .copy_from_slice(&bytes);
    Ok(out)
}

/// 在原 edit list 前加入空白时间，保留编码器裁剪和播放速率。
pub(super) fn delay_track(trak: &[u8], delay: u64, presentation: u64) -> io::Result<Vec<u8>> {
    let previous = children(trak)?.into_iter().find(|c| &c[4..8] == b"edts");
    let mut entries = Vec::new();
    if let Some(edts) = previous {
        let elst = child(edts, b"elst")?;
        let h = header(elst)?;
        let version = *elst.get(h).ok_or_else(|| invalid("short edit list"))?;
        let width = match version {
            0 => 12,
            1 => 20,
            _ => return Err(invalid("edit list 版本不受支持")),
        };
        let count = u32_at(elst, h + 4)? as usize;
        let remaining = elst.len() - h - 8;
        if count != remaining / width || remaining % width != 0 {
            return Err(invalid("edit list 长度无效"));
        }
        for n in 0..count {
            let p = h + 8 + n * width;
            let (duration, time, rate) = if version == 1 {
                (
                    u64_at(elst, p)?,
                    u64_at(elst, p + 8)? as i64,
                    u32_at(elst, p + 16)?,
                )
            } else {
                (
                    u64::from(u32_at(elst, p)?),
                    i64::from(u32_at(elst, p + 4)? as i32),
                    u32_at(elst, p + 8)?,
                )
            };
            entries.push((duration, time, rate));
        }
    } else {
        entries.push((presentation, 0, 0x00010000));
    }
    let mut edits = vec![1, 0, 0, 0];
    edits.extend_from_slice(
        &u32::try_from(entries.len() + 1)
            .map_err(|_| invalid("edit list 条目过多"))?
            .to_be_bytes(),
    );
    for (duration, time, rate) in std::iter::once((delay, -1i64, 0x00010000)).chain(entries) {
        edits.extend_from_slice(&duration.to_be_bytes());
        edits.extend_from_slice(&time.to_be_bytes());
        edits.extend_from_slice(&rate.to_be_bytes());
    }
    let elst = atom(b"elst", &edits)?;
    if let Some(edts) = previous {
        replace(trak, b"edts", &replace(edts, b"elst", &elst)?)
    } else {
        let mut body = trak[header(trak)?..].to_vec();
        body.extend_from_slice(&atom(b"edts", &elst)?);
        atom(b"trak", &body)
    }
}

impl Mp4Builder {
    /// 根据采样信息和输出偏移生成 moov。
    pub(super) fn build_moov(&self, offsets: &[Vec<u64>]) -> io::Result<Vec<u8>> {
        let movie_scale = time_scale(&self.mvhd)?;
        let mut body = Vec::new();
        let mut movie_duration = 0;
        let mut traks = Vec::new();
        for (index, t) in self.tracks.iter().enumerate() {
            let track_duration: u64 = t.samples.iter().map(|s| s.duration as u64).sum();
            let scaled = u64::try_from(
                (track_duration as u128 * movie_scale as u128).div_ceil(t.timescale as u128),
            )
            .map_err(|_| invalid("duration overflow"))?;
            let mdia = child(&t.trak, b"mdia")?;
            let delayed = if self.audio_delay_ms != 0 {
                let handler = child(mdia, b"hdlr")?;
                (self.audio_delay_ms > 0 && handler.get(16..20) == Some(b"soun"))
                    || (self.audio_delay_ms < 0 && handler.get(16..20) == Some(b"vide"))
            } else {
                false
            };
            let delay = if delayed {
                (u64::from(self.audio_delay_ms.unsigned_abs()) * u64::from(movie_scale))
                    .div_ceil(1000)
            } else {
                0
            };
            let presentation_duration = scaled
                .checked_add(delay)
                .ok_or_else(|| invalid("presentation duration overflow"))?;
            movie_duration = movie_duration.max(presentation_duration);
            let minf = child(mdia, b"minf")?;
            let stbl = child(minf, b"stbl")?;
            let mut tables = child(stbl, b"stsd")?.to_vec(); // preserve codec configuration verbatim
            let durations: Vec<u32> = t.samples.iter().map(|s| s.duration).collect();
            tables.extend_from_slice(&run_table(b"stts", 0, &durations)?);
            if t.samples.iter().any(|s| s.composition_offset != 0) {
                tables.extend_from_slice(&run_table(
                    b"ctts",
                    1,
                    &t.samples
                        .iter()
                        .map(|s| s.composition_offset as u32)
                        .collect::<Vec<_>>(),
                )?);
            }
            let mut stsc = vec![0; 4];
            for n in [1u32, 1, 1, 1] {
                stsc.extend_from_slice(&n.to_be_bytes());
            }
            tables.extend_from_slice(&atom(b"stsc", &stsc)?);
            let count = u32::try_from(t.samples.len()).map_err(|_| invalid("too many samples"))?;
            let mut stsz = vec![0; 8];
            stsz.extend_from_slice(&count.to_be_bytes());
            for s in &t.samples {
                stsz.extend_from_slice(&s.size.to_be_bytes());
            }
            tables.extend_from_slice(&atom(b"stsz", &stsz)?);
            let mut co64 = vec![0; 4];
            co64.extend_from_slice(&count.to_be_bytes());
            for offset in &offsets[index] {
                co64.extend_from_slice(&offset.to_be_bytes());
            }
            tables.extend_from_slice(&atom(b"co64", &co64)?);
            if t.samples.iter().any(|s| !s.sync) {
                let sync: Vec<u32> = t
                    .samples
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.sync)
                    .map(|(i, _)| i as u32 + 1)
                    .collect();
                let mut stss = vec![0; 4];
                stss.extend_from_slice(&(sync.len() as u32).to_be_bytes());
                for n in sync {
                    stss.extend_from_slice(&n.to_be_bytes());
                }
                tables.extend_from_slice(&atom(b"stss", &stss)?);
            }
            let minf = replace(minf, b"stbl", &atom(b"stbl", &tables)?)?;
            let mdia = replace(mdia, b"minf", &minf)?;
            let mdia = replace(
                &mdia,
                b"mdhd",
                &duration(child(&mdia, b"mdhd")?, track_duration, false)?,
            )?;
            let mut trak = replace(&t.trak, b"mdia", &mdia)?;
            // DASH uses an open-ended edit (duration=0) for encoder delay.
            // Ordinary MP4 interprets zero as an empty presentation. Preserve
            // the media-time offset but give the edit its now-known duration.
            if let Some(edts) = children(&trak)?.into_iter().find(|c| &c[4..8] == b"edts") {
                let elst = child(edts, b"elst")?;
                let h = header(elst)?;
                let version = *elst.get(h).ok_or_else(|| invalid("short edit list"))?;
                if version > 1 || u32_at(elst, h + 4)? != 1 {
                    return Err(invalid("complex edit lists unsupported"));
                }
                let mut edit = elst.to_vec();
                let p = h + 8;
                if version == 0 && u32_at(elst, p)? == 0 {
                    edit[p..p + 4].copy_from_slice(
                        &u32::try_from(scaled)
                            .map_err(|_| invalid("edit duration overflow"))?
                            .to_be_bytes(),
                    );
                } else if version == 1 && u64_at(elst, p)? == 0 {
                    edit[p..p + 8].copy_from_slice(&scaled.to_be_bytes());
                }
                trak = replace(&trak, b"edts", &replace(edts, b"elst", &edit)?)?;
            }
            if delay != 0 {
                trak = delay_track(&trak, delay, scaled)?;
            }
            let mut tkhd = duration(child(&trak, b"tkhd")?, presentation_duration, true)?;
            let h = header(&tkhd)?;
            let p = h + if tkhd[h] == 1 { 20 } else { 12 };
            tkhd[p..p + 4].copy_from_slice(&(index as u32 + 1).to_be_bytes());
            traks.extend_from_slice(&replace(&trak, b"tkhd", &tkhd)?);
        }
        let mut mvhd = duration(&self.mvhd, movie_duration, false)?;
        let n = mvhd.len();
        mvhd[n - 4..].copy_from_slice(&(self.tracks.len() as u32 + 1).to_be_bytes());
        body.extend_from_slice(&mvhd);
        body.extend_from_slice(&traks);
        atom(b"moov", &body)
    }
}

/// 将连续相同的采样值合并为游程表。
fn run_table(kind: &[u8; 4], version: u8, values: &[u32]) -> io::Result<Vec<u8>> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for &value in values {
        if let Some((count, last)) = runs.last_mut().filter(|(_, last)| *last == value) {
            *count += 1;
            let _ = last;
        } else {
            runs.push((1, value));
        }
    }
    let mut body = vec![version, 0, 0, 0];
    body.extend_from_slice(&(runs.len() as u32).to_be_bytes());
    for (count, value) in runs {
        body.extend_from_slice(&count.to_be_bytes());
        body.extend_from_slice(&value.to_be_bytes());
    }
    atom(kind, &body)
}
