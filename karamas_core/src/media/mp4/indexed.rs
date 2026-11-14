//! 从分片样本元数据构建可随机读取的普通 MP4，不重新编码媒体。
use bytes::Bytes;
use std::{io, ops::Range};

use super::{
    boxes::{child, children, header, invalid, u32_at},
    samples::{Sample, fragment_samples},
};
use layout::Extent;
use moov::time_scale;

mod layout;
mod moov;
mod progressive;
pub(crate) use progressive::prepare as prepare_progressive;
pub use layout::{ReadPart, VirtualMp4};

struct Track {
    source: usize,
    id: u32,
    trak: Vec<u8>,
    timescale: u32,
    defaults: [u32; 3],
    samples: Vec<Sample>,
}

/// Metadata-only builder. `source` identifies an immutable upstream file.
pub struct Mp4Builder {
    ftyp: Vec<u8>,
    mvhd: Vec<u8>,
    tracks: Vec<Track>,
    sample_count: usize,
    audio_delay_ms: i32,
}

impl Mp4Builder {
    /// 使用首个媒体源的初始化信息创建 MP4 索引构建器。
    pub fn new(ftyp: &[u8], moov: &[u8], source: usize) -> io::Result<Self> {
        let mut result = Self {
            ftyp: ftyp.to_vec(),
            mvhd: child(moov, b"mvhd")?.to_vec(),
            tracks: Vec::new(),
            sample_count: 0,
            audio_delay_ms: 0,
        };
        result.add_source(moov, source)?;
        Ok(result)
    }

    /// 正值延迟音频，负值延迟视频；使用 edit list 保留编码数据。
    pub fn set_audio_delay_ms(&mut self, milliseconds: i32) {
        self.audio_delay_ms = milliseconds;
    }

    /// 添加媒体源的轨道信息和默认采样参数。
    pub fn add_source(&mut self, moov: &[u8], source: usize) -> io::Result<()> {
        let trexes = children(child(moov, b"mvex")?)?;
        for trak in children(moov)?.into_iter().filter(|c| &c[4..8] == b"trak") {
            let tkhd = child(trak, b"tkhd")?;
            let h = header(tkhd)?;
            let version = *tkhd.get(h).ok_or_else(|| invalid("short track header"))?;
            if version > 1 {
                return Err(invalid("unknown track header version"));
            }
            let id = u32_at(tkhd, h + if version == 1 { 20 } else { 12 })?;
            if self.tracks.iter().any(|t| t.source == source && t.id == id) {
                return Err(invalid("duplicate source track"));
            }
            let trex = trexes
                .iter()
                .find(|c| {
                    &c[4..8] == b"trex" && u32_at(c, header(c).unwrap_or(8) + 4).ok() == Some(id)
                })
                .ok_or_else(|| invalid("missing track defaults"))?;
            let th = header(trex)?;
            if u32_at(trex, th + 8)? != 1 {
                return Err(invalid("multiple sample descriptions unsupported"));
            }
            self.tracks.push(Track {
                source,
                id,
                trak: trak.to_vec(),
                timescale: time_scale(child(child(trak, b"mdia")?, b"mdhd")?)?,
                defaults: [
                    u32_at(trex, th + 12)?,
                    u32_at(trex, th + 16)?,
                    u32_at(trex, th + 20)?,
                ],
                samples: Vec::new(),
            });
        }
        Ok(())
    }

    /// Parse sample metadata without reading the following media payload.
    pub fn add_fragment(&mut self, source: usize, moof_offset: u64, moof: &[u8]) -> io::Result<()> {
        for traf in children(moof)?.into_iter().filter(|c| &c[4..8] == b"traf") {
            let tfhd = child(traf, b"tfhd")?;
            let h = header(tfhd)?;
            let id = u32_at(tfhd, h + 4)?;
            let t = self
                .tracks
                .iter_mut()
                .find(|t| t.source == source && t.id == id)
                .ok_or_else(|| invalid("unknown fragment track"))?;
            let samples = fragment_samples(traf, moof_offset, t.defaults)?;
            let expected = t
                .samples
                .last()
                .map_or(0, |s| s.decode_time + u64::from(s.duration));
            if samples
                .first()
                .is_some_and(|sample| sample.decode_time != expected)
            {
                return Err(invalid("discontinuous decode timeline unsupported"));
            }
            self.sample_count = self
                .sample_count
                .checked_add(samples.len())
                .ok_or_else(|| invalid("sample count overflow"))?;
            if self.sample_count > 1_000_000 {
                return Err(invalid("sample index exceeds memory budget"));
            }
            t.samples.extend(samples);
        }
        Ok(())
    }

    /// Validate every newly indexed sample against its segment's mdat payload.
    /// On any parse/validation error the builder must be discarded.
    pub fn add_fragment_in_media(
        &mut self,
        source: usize,
        offset: u64,
        moof: &[u8],
        media: Range<u64>,
    ) -> io::Result<()> {
        let before: Vec<_> = self.tracks.iter().map(|t| t.samples.len()).collect();
        self.add_fragment(source, offset, moof)?;
        for (track, count) in self.tracks.iter().zip(before) {
            for sample in &track.samples[count..] {
                let end = sample
                    .offset
                    .checked_add(sample.size as u64)
                    .ok_or_else(|| invalid("sample end overflow"))?;
                if sample.offset < media.start || end > media.end {
                    return Err(invalid("sample exceeds its media payload"));
                }
            }
        }
        Ok(())
    }

    /// 生成可定位 MP4 的头部和源文件字节映射。
    pub fn finish(self) -> io::Result<VirtualMp4> {
        if self.tracks.is_empty() || self.tracks.iter().any(|t| t.samples.is_empty()) {
            return Err(invalid("empty track"));
        }
        let mut order: Vec<(usize, usize)> = self
            .tracks
            .iter()
            .enumerate()
            .flat_map(|(t, track)| (0..track.samples.len()).map(move |s| (t, s)))
            .collect();
        order.sort_by(|&(a, x), &(b, y)| {
            let left =
                self.tracks[a].samples[x].decode_time as u128 * self.tracks[b].timescale as u128;
            let right =
                self.tracks[b].samples[y].decode_time as u128 * self.tracks[a].timescale as u128;
            left.cmp(&right).then(a.cmp(&b))
        });
        let mut offsets: Vec<Vec<u64>> = self
            .tracks
            .iter()
            .map(|t| vec![0; t.samples.len()])
            .collect();
        // co64 always has fixed-width entries, so filling offsets cannot change moov size.
        let initial_moov = self.build_moov(&offsets)?;
        let payload_start = (self.ftyp.len() + initial_moov.len() + 16) as u64;
        let mut position = payload_start;
        let mut extents: Vec<Extent> = Vec::new();
        for (t, s) in order {
            let sample = &self.tracks[t].samples[s];
            offsets[t][s] = position;
            let end = position
                .checked_add(sample.size as u64)
                .ok_or_else(|| invalid("output too large"))?;
            extents.push(Extent {
                output: position..end,
                source: self.tracks[t].source,
                source_start: sample.offset,
            });
            position = end;
        }
        let moov = self.build_moov(&offsets)?;
        // Hints only: the receiver still uses the exact MP4 sample tables for
        // seeking. Start at a preceding video keyframe, never at an arbitrary
        // byte estimate based on average bitrate.
        let video = self
            .tracks
            .iter()
            .position(|t| {
                child(&t.trak, b"mdia")
                    .and_then(|m| child(m, b"hdlr"))
                    .is_ok_and(|h| h.get(16..20) == Some(b"vide"))
            })
            .unwrap_or(0);
        let seek_points = self.tracks[video]
            .samples
            .iter()
            .enumerate()
            .filter(|(_, s)| s.sync)
            .map(|(i, s)| {
                (
                    (s.decode_time as u128 * 1000 / self.tracks[video].timescale as u128) as u64
                        + u64::from(self.audio_delay_ms.min(0).unsigned_abs()),
                    offsets[video][i],
                )
            })
            .collect();
        assert_eq!(moov.len(), initial_moov.len());
        let mut prefix = self.ftyp;
        prefix.extend_from_slice(&moov);
        prefix.extend_from_slice(&1u32.to_be_bytes());
        prefix.extend_from_slice(b"mdat");
        prefix.extend_from_slice(&(position - payload_start + 16).to_be_bytes());
        Ok(VirtualMp4 {
            prefix: Bytes::from(prefix),
            len: position,
            extents,
            seek_points,
        })
    }
}

#[cfg(test)]
mod tests;
