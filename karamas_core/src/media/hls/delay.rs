use super::invalid;
use crate::media::{
    mp4::{
        boxes::{boxes, child, children, u32_at, u64_at},
        fragmented::{delay_fragment, track_timescale},
    },
    upstream::fetch_range,
};
use bytes::Bytes;
use std::{
    collections::HashMap,
    io,
    ops::Range,
    sync::{Arc, Mutex},
};

struct TimestampPatch {
    offset: u64,
    before: Bytes,
    after: Bytes,
}

pub(crate) struct FragmentDelay {
    milliseconds: u32,
    timescale: u32,
    segments: Vec<Range<u64>>,
    patches: Mutex<HashMap<u64, Arc<[TimestampPatch]>>>,
}

impl FragmentDelay {
    pub(super) fn new(
        milliseconds: u32,
        init: &[u8],
        segments: &[(Range<u64>, f64)],
    ) -> io::Result<Self> {
        let moov = boxes(init)?
            .into_iter()
            .find(|part| &part[4..8] == b"moov")
            .ok_or_else(|| invalid("HLS 缺少初始化 moov"))?;
        Ok(Self {
            milliseconds,
            timescale: track_timescale(moov)?,
            segments: segments.iter().map(|(range, _)| range.clone()).collect(),
            patches: Mutex::default(),
        })
    }

    pub(super) async fn apply(
        &self,
        client: &reqwest::Client,
        url: &str,
        total: u64,
        range: Range<u64>,
        bytes: Bytes,
    ) -> io::Result<Bytes> {
        let first = self
            .segments
            .partition_point(|segment| segment.end <= range.start);
        let mut output = None;
        for segment in self.segments[first..]
            .iter()
            .take_while(|s| s.start < range.end)
        {
            let cached = self.patches.lock().unwrap().get(&segment.start).cloned();
            let needs_cache = cached.is_none();
            let patches = if let Some(cached) = cached {
                cached
            } else {
                // Range 可能从分片中间开始，需要单独读取该分片的元数据。
                let prefix = if segment.start >= range.start {
                    bytes.slice(
                        (segment.start - range.start) as usize
                            ..(segment.end.min(range.end) - range.start) as usize,
                    )
                } else {
                    Bytes::new()
                };
                let (offset, moof) = load_moof(client, url, total, segment, prefix).await?;
                let shifted = delay_fragment(&moof, self.timescale, self.milliseconds)?;
                let mut patches = Vec::new();
                for traf in children(&moof)?.into_iter().filter(|b| &b[4..8] == b"traf") {
                    let tfdt = child(traf, b"tfdt")?;
                    let start = tfdt.as_ptr() as usize - moof.as_ptr() as usize;
                    patches.push(TimestampPatch {
                        offset: offset + start as u64,
                        before: Bytes::copy_from_slice(tfdt),
                        after: Bytes::copy_from_slice(&shifted[start..start + tfdt.len()]),
                    });
                }
                patches.into()
            };
            for patch in patches.iter() {
                let start = range.start.max(patch.offset);
                let end = range.end.min(patch.offset + patch.before.len() as u64);
                if start >= end {
                    continue;
                }
                let target = (start - range.start) as usize..(end - range.start) as usize;
                let source = (start - patch.offset) as usize..(end - patch.offset) as usize;
                if bytes[target.clone()] != patch.before[source.clone()] {
                    return Err(invalid("HLS 分片时间戳已变化，请重新播放"));
                }
                output.get_or_insert_with(|| bytes.to_vec())[target]
                    .copy_from_slice(&patch.after[source]);
            }
            if needs_cache {
                self.patches.lock().unwrap().insert(segment.start, patches);
            }
        }
        Ok(output.map(Bytes::from).unwrap_or(bytes))
    }
}

async fn load_moof(
    client: &reqwest::Client,
    url: &str,
    total: u64,
    segment: &Range<u64>,
    mut prefix: Bytes,
) -> io::Result<(u64, Bytes)> {
    if prefix.len() < 16 {
        let end = segment.start + (segment.end - segment.start).min(16 * 1024);
        prefix = fetch_range(client, url, segment.start..end, total).await?;
    }
    let read = |range: Range<u64>| {
        let local = (range.start - segment.start) as usize..(range.end - segment.start) as usize;
        let prefix = &prefix;
        async move {
            if local.end <= prefix.len() {
                Ok(prefix.slice(local))
            } else {
                fetch_range(client, url, range, total).await
            }
        }
    };
    let mut position = segment.start;
    let mut moof = None;
    for _ in 0..17 {
        let remaining = segment.end - position;
        if remaining < 8 {
            return Err(invalid("HLS 分片头不完整"));
        }
        let head = read(position..position + remaining.min(16)).await?;
        let size = u32_at(&head, 0)?;
        let (length, header) = match size {
            1 => (u64_at(&head, 8)?, 16),
            size => (u64::from(size), 8),
        };
        if length < header || length > remaining {
            return Err(invalid("HLS 分片 box 超出索引范围"));
        }
        let end = position + length;
        match (&head[4..8], &moof) {
            (b"moof", None) if length <= 16 * 1024 * 1024 => moof = Some(position..end),
            (b"mdat", Some(metadata)) if end == segment.end => {
                return Ok((metadata.start, read(metadata.clone()).await?));
            }
            (b"moof" | b"mdat", _) | (_, Some(_)) => {
                return Err(invalid("HLS 分片须包含一组 moof/mdat"));
            }
            (_, None) => (),
        }
        position = end;
    }
    Err(invalid("HLS 分片缺少 moof/mdat"))
}
