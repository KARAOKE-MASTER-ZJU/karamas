//! 虚拟 MP4 的字节映射、源范围校验和 seek 预读范围。
use crate::media::mp4::boxes::invalid;
use bytes::Bytes;
use std::{io, ops::Range};

pub(super) struct Extent {
    pub(super) output: Range<u64>,
    pub(super) source: usize,
    pub(super) source_start: u64,
}
pub struct VirtualMp4 {
    pub prefix: Bytes,
    pub len: u64,
    pub(super) extents: Vec<Extent>,
    pub(super) seek_points: Vec<(u64, u64)>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReadPart {
    Header(Range<usize>),
    Source { source: usize, range: Range<u64> },
}

impl VirtualMp4 {
    /// A bounded warm-up window beginning at the preceding keyframe. This is
    /// not a replacement for the renderer's precise timestamp/byte mapping.
    pub fn seek_warmup_range(&self, seconds: u32) -> Range<u64> {
        let target = u64::from(seconds) * 1000;
        let index = self.seek_points.partition_point(|(ms, _)| *ms <= target);
        let start = self
            .seek_points
            .get(index.saturating_sub(1))
            .map_or(self.prefix.len() as u64, |(_, offset)| *offset);
        start..start.saturating_add(2 * 1024 * 1024).min(self.len)
    }
    /// 校验采样映射是否位于对应媒体源范围内。
    pub fn validate_sources(&self, lengths: &[u64]) -> io::Result<()> {
        for extent in &self.extents {
            let length = *lengths
                .get(extent.source)
                .ok_or_else(|| invalid("missing media source"))?;
            let end = extent
                .source_start
                .checked_add(extent.output.end - extent.output.start)
                .ok_or_else(|| invalid("source range overflow"))?;
            if end > length {
                return Err(invalid("sample data exceeds source file"));
            }
        }
        Ok(())
    }
    /// Translate an exact half-open output range into source ranges, without I/O.
    pub fn read_plan(&self, range: Range<u64>) -> io::Result<Vec<ReadPart>> {
        if range.start > range.end || range.end > self.len {
            return Err(invalid("range outside virtual MP4"));
        }
        let mut out = Vec::new();
        let mut p = range.start;
        let header_end = (self.prefix.len() as u64).min(range.end);
        if p < header_end {
            out.push(ReadPart::Header(p as usize..header_end as usize));
            p = header_end;
        }
        let i = self.extents.partition_point(|e| e.output.end <= p);
        for extent in &self.extents[i..] {
            if p >= range.end {
                break;
            }
            let end = extent.output.end.min(range.end);
            let source_start = extent.source_start + p - extent.output.start;
            let source_end = source_start + end - p;
            if let Some(ReadPart::Source { source, range: previous }) = out.last_mut().filter(|part| matches!(part, ReadPart::Source { source, range } if *source == extent.source && range.end == source_start)) {
                let _ = source;
                previous.end = source_end;
            } else { out.push(ReadPart::Source { source: extent.source, range: source_start..source_end }); }
            p = end;
        }
        Ok(out)
    }
}
