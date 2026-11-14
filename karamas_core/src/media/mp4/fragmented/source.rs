//! 上游 fMP4 流读取、分片索引和按时间定位。
use super::{
    boxes::{atom_type, box_header_len, box_size_prefix, invalid, read_u16, read_u32, read_u64},
    metadata::fragment_decode_time,
};
use bytes::{Bytes, BytesMut};
use futures_util::{Stream, StreamExt};
use std::{io, pin::Pin};

const MAX_BOX_SIZE: usize = 64 * 1024 * 1024;

type UpstreamStream = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

struct BoxReader {
    stream: UpstreamStream,
    buffered: BytesMut,
    eof: bool,
}

impl BoxReader {
    /// 从媒体源起始位置创建 MP4 box 读取器。
    async fn open(client: &reqwest::Client, url: &str, track: &'static str) -> io::Result<Self> {
        Self::open_at(client, url, track, None).await
    }

    /// 从媒体源指定偏移创建 MP4 box 读取器。
    async fn open_at(
        client: &reqwest::Client,
        url: &str,
        track: &'static str,
        offset: Option<u64>,
    ) -> io::Result<Self> {
        let endpoint = safe_endpoint(url);
        if let Some(offset) = offset {
            log::info!(target: "DLNA1080", "连接 {} 上游定位流: {}, offset={}B", track, endpoint, offset);
        } else {
            log::info!(target: "DLNA1080", "连接 {} 上游: {}", track, endpoint);
        }
        let mut request = client
            .get(url)
            .header("User-Agent", "Mozilla/5.0")
            .header("Referer", crate::media::upstream::referer(url));
        if let Some(offset) = offset {
            request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
        }
        let response = request
            .send()
            .await
            .map_err(|err| {
                log::error!(target: "DLNA1080", "连接 {} 上游失败: {}, error={}", track, endpoint, err);
                io::Error::other(err)
            })?
            .error_for_status()
            .map_err(|err| {
                log::error!(target: "DLNA1080", "{} 上游 HTTP 失败: {}, error={}", track, endpoint, err);
                io::Error::other(err)
            })?;
        if offset.is_some() && response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(invalid(format!(
                "{track} upstream ignored byte range: status={}",
                response.status()
            )));
        }
        log::info!(target: "DLNA1080", "{} 上游连接成功: status={}", track, response.status());
        Ok(Self {
            stream: Box::pin(response.bytes_stream()),
            buffered: BytesMut::new(),
            eof: false,
        })
    }

    /// 读取下一个完整 MP4 box，流结束时返回 None。
    async fn next_box(&mut self) -> io::Result<Option<Bytes>> {
        loop {
            if self.buffered.len() >= 8 {
                let header_len = if read_u32(&self.buffered, 0)? == 1 {
                    16
                } else {
                    8
                };
                if self.buffered.len() >= header_len {
                    let size = box_size_prefix(&self.buffered)?;
                    if size < header_len || size > MAX_BOX_SIZE {
                        return Err(invalid(format!("invalid fMP4 box size {size}")));
                    }
                    if self.buffered.len() >= size {
                        return Ok(Some(self.buffered.split_to(size).freeze()));
                    }
                }
            }

            if self.eof {
                if self.buffered.is_empty() {
                    return Ok(None);
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated fMP4 box",
                ));
            }
            match self.stream.next().await {
                Some(Ok(chunk)) => self.buffered.extend_from_slice(&chunk),
                Some(Err(err)) => return Err(io::Error::other(err)),
                None => self.eof = true,
            }
        }
    }
}

pub(super) struct TrackSource {
    reader: BoxReader,
    pub(super) moov: Bytes,
    pending_moof: Option<Bytes>,
    pub(super) sidx: Option<SidxInfo>,
    url: String,
    track: &'static str,
}

#[derive(Clone, Debug)]
pub(super) struct SidxInfo {
    pub(super) referenced_bytes: u64,
    pub(super) duration: u64,
    pub(super) timescale: u32,
    earliest_presentation_time: u64,
    first_media_offset: u64,
    entries: Vec<SidxEntry>,
}

#[derive(Clone, Copy, Debug)]
struct SidxEntry {
    referenced_size: u32,
    subsegment_duration: u32,
}

impl TrackSource {
    /// 读取单轨媒体的初始化信息和分片索引。
    pub(super) async fn open(
        client: &reqwest::Client,
        url: &str,
        track: &'static str,
    ) -> io::Result<(Bytes, Self)> {
        let mut reader = BoxReader::open(client, url, track).await?;
        let mut ftyp = None;
        let mut moov = None;
        let mut pending_moof = None;
        let mut sidx = None;
        let mut bytes_before_atom = 0u64;

        for _ in 0..32 {
            let Some(atom) = reader.next_box().await? else {
                break;
            };
            let atom_len = atom.len() as u64;
            match atom_type(&atom)? {
                b"ftyp" => ftyp = Some(atom),
                b"moov" => moov = Some(atom),
                b"sidx" => {
                    sidx = parse_sidx(&atom).ok().map(|mut index| {
                        index.first_media_offset =
                            bytes_before_atom + atom.len() as u64 + index.first_media_offset;
                        index
                    });
                }
                b"moof" => {
                    pending_moof = Some(atom);
                    break;
                }
                _ => {}
            }
            bytes_before_atom += atom_len;
        }

        let ftyp = ftyp.ok_or_else(|| invalid("fMP4 source has no ftyp"))?;
        let moov = moov.ok_or_else(|| invalid("fMP4 source has no moov"))?;
        log::info!(
            target: "DLNA1080",
            "{} 初始化段读取完成: ftyp={}B, moov={}B, indexed_media={}B",
            track,
            ftyp.len(),
            moov.len(),
            sidx.as_ref().map_or_else(|| "unknown".to_string(), |info| info.referenced_bytes.to_string())
        );
        Ok((
            ftyp,
            Self {
                reader,
                moov,
                pending_moof,
                sidx,
                url: url.to_owned(),
                track,
            },
        ))
    }

    /// 根据分片索引重新打开接近目标时间的媒体流。
    pub(super) async fn seek_to(
        &mut self,
        client: &reqwest::Client,
        start_secs: u32,
    ) -> io::Result<()> {
        let Some(sidx) = self.sidx.as_ref() else {
            return Err(invalid("fMP4 source has no sidx seek index"));
        };
        let Some((offset, segment_secs)) = sidx_offset_for_start(sidx, start_secs) else {
            return Err(invalid(format!(
                "seek position {start_secs}s is outside sidx index"
            )));
        };
        let reader = BoxReader::open_at(client, &self.url, self.track, Some(offset)).await?;
        self.reader = reader;
        self.pending_moof = None;
        log::info!(
            target: "DLNA1080",
            "{} 定位索引命中: requested={}s, segment_start={:.3}s, upstream_offset={}B",
            self.track,
            start_secs,
            segment_secs,
            offset
        );
        Ok(())
    }

    /// 读取下一个包含 moof 和 mdat 的媒体分片。
    pub(super) async fn next_fragment(&mut self) -> io::Result<Option<Fragment>> {
        let moof = if let Some(moof) = self.pending_moof.take() {
            moof
        } else {
            loop {
                let Some(atom) = self.reader.next_box().await? else {
                    return Ok(None);
                };
                if atom_type(&atom)? == b"moof" {
                    break atom;
                }
            }
        };
        let decode_time = fragment_decode_time(&moof)?;
        loop {
            let atom =
                self.reader.next_box().await?.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "moof without mdat")
                })?;
            if atom_type(&atom)? == b"mdat" {
                return Ok(Some(Fragment {
                    moof,
                    mdat: atom,
                    decode_time,
                }));
            }
        }
    }
}

pub(super) struct Fragment {
    pub(super) moof: Bytes,
    pub(super) mdat: Bytes,
    pub(super) decode_time: u64,
}

fn safe_endpoint(url: &str) -> String {
    reqwest::Url::parse(url)
        .map(|url| {
            format!(
                "{}://{}{}",
                url.scheme(),
                url.host_str().unwrap_or("unknown"),
                url.path()
            )
        })
        .unwrap_or_else(|_| "<invalid-url>".to_string())
}

/// 解析分片索引中的偏移、大小和时长。
fn parse_sidx(sidx: &[u8]) -> io::Result<SidxInfo> {
    let header = box_header_len(sidx)?;
    let version = *sidx.get(header).ok_or_else(|| invalid("short sidx"))?;
    let timescale = read_u32(sidx, header + 8)?;
    if timescale == 0 {
        return Err(invalid("zero sidx timescale"));
    }
    let mut offset = header + 4 + 4 + 4;
    let earliest_presentation_time = if version == 0 {
        read_u32(sidx, offset)? as u64
    } else {
        read_u64(sidx, offset)?
    };
    offset += if version == 0 { 4 } else { 8 };
    let first_media_offset = if version == 0 {
        read_u32(sidx, offset)? as u64
    } else {
        read_u64(sidx, offset)?
    };
    offset += if version == 0 { 4 } else { 8 };
    offset += 2;
    let count = read_u16(sidx, offset)? as usize;
    offset += 2;
    let mut total = 0u64;
    let mut duration = 0u64;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let reference = read_u32(sidx, offset)?;
        if reference >> 31 != 0 {
            return Err(invalid("nested sidx reference unsupported"));
        }
        let referenced_size = reference & 0x7fff_ffff;
        let subsegment_duration = read_u32(sidx, offset + 4)?;
        total += referenced_size as u64;
        duration += subsegment_duration as u64;
        entries.push(SidxEntry {
            referenced_size,
            subsegment_duration,
        });
        offset += 12;
    }
    Ok(SidxInfo {
        referenced_bytes: total,
        duration,
        timescale,
        earliest_presentation_time,
        first_media_offset,
        entries,
    })
}

/// 查找目标时间及之后最近分片的源文件偏移。
fn sidx_offset_for_start(sidx: &SidxInfo, start_secs: u32) -> Option<(u64, f64)> {
    let target_time = (u128::from(start_secs) * u128::from(sidx.timescale)) as u64;
    let mut offset = sidx.first_media_offset;
    let mut segment_time = sidx.earliest_presentation_time;
    for entry in &sidx.entries {
        if segment_time >= target_time {
            return Some((offset, segment_time as f64 / sidx.timescale as f64));
        }
        offset += u64::from(entry.referenced_size);
        segment_time += u64::from(entry.subsegment_duration);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::boxes::make_box;
    use super::*;

    #[test]
    fn log_endpoint_does_not_expose_query_credentials() {
        let endpoint =
            safe_endpoint("https://upos-sz.example.com/path/video.m4s?deadline=123&token=secret");
        assert_eq!(endpoint, "https://upos-sz.example.com/path/video.m4s");
        assert!(!endpoint.contains("secret"));
    }

    #[test]
    fn parses_sidx_media_size_and_full_duration() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0, 0, 0, 0]); // version + flags
        body.extend_from_slice(&1u32.to_be_bytes()); // reference_ID
        body.extend_from_slice(&1_000u32.to_be_bytes()); // timescale
        body.extend_from_slice(&0u32.to_be_bytes()); // earliest_presentation_time
        body.extend_from_slice(&0u32.to_be_bytes()); // first_offset
        body.extend_from_slice(&0u16.to_be_bytes()); // reserved
        body.extend_from_slice(&2u16.to_be_bytes()); // reference_count
        for (size, duration) in [(120u32, 5_000u32), (80u32, 4_000u32)] {
            body.extend_from_slice(&size.to_be_bytes());
            body.extend_from_slice(&duration.to_be_bytes());
            body.extend_from_slice(&0u32.to_be_bytes()); // SAP flags
        }
        let sidx = make_box(b"sidx", &[&body]).unwrap();

        let parsed = parse_sidx(&sidx).unwrap();
        assert_eq!(parsed.referenced_bytes, 200);
        assert_eq!(parsed.duration, 9_000);
        assert_eq!(parsed.timescale, 1_000);
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(sidx_offset_for_start(&parsed, 0), Some((0, 0.0)));
        assert_eq!(sidx_offset_for_start(&parsed, 5), Some((120, 5.0)));
        assert_eq!(sidx_offset_for_start(&parsed, 9), None);
    }
}
