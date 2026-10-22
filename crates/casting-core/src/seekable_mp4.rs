//! An immutable ordinary MP4 backed by byte ranges in fragmented source files.
//!
//! Only initialization and fragment headers are needed to construct the layout.
//! Encoded samples remain in their original files. Each sample is one MP4 chunk;
//! adjacent source ranges are coalesced when answering reads. No codecs run here.
use bytes::Bytes;
use std::{io, ops::Range};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn u32_at(b: &[u8], p: usize) -> io::Result<u32> {
    Ok(u32::from_be_bytes(
        b.get(p..p + 4)
            .ok_or_else(|| invalid("short u32"))?
            .try_into()
            .unwrap(),
    ))
}
fn u64_at(b: &[u8], p: usize) -> io::Result<u64> {
    Ok(u64::from_be_bytes(
        b.get(p..p + 8)
            .ok_or_else(|| invalid("short u64"))?
            .try_into()
            .unwrap(),
    ))
}
fn atom(kind: &[u8; 4], body: &[u8]) -> io::Result<Vec<u8>> {
    let size = u32::try_from(
        body.len()
            .checked_add(8)
            .ok_or_else(|| invalid("box overflow"))?,
    )
    .map_err(|_| invalid("box too large"))?;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    Ok(out)
}
fn header(b: &[u8]) -> io::Result<usize> {
    if b.len() < 8 {
        return Err(invalid("short box"));
    }
    let size = if u32_at(b, 0)? == 1 { 16 } else { 8 };
    if b.len() < size {
        return Err(invalid("short extended box header"));
    }
    Ok(size)
}
fn children(b: &[u8]) -> io::Result<Vec<&[u8]>> {
    let mut p = header(b)?;
    let declared = match u32_at(b, 0)? {
        0 => b.len() as u64,
        1 => u64_at(b, 8)?,
        n => n as u64,
    };
    if declared != b.len() as u64 {
        return Err(invalid("container size mismatch"));
    }
    let mut out = Vec::new();
    while p < b.len() {
        let size = match u32_at(b, p)? {
            0 => b.len() - p,
            1 => usize::try_from(u64_at(b, p + 8)?).map_err(|_| invalid("box overflow"))?,
            n => n as usize,
        };
        let end = p.checked_add(size).ok_or_else(|| invalid("box overflow"))?;
        let child = b
            .get(p..end)
            .ok_or_else(|| invalid("child exceeds parent"))?;
        if size < header(child)? {
            return Err(invalid("invalid child size"));
        }
        out.push(child);
        p = end;
    }
    Ok(out)
}
fn child<'a>(b: &'a [u8], kind: &[u8; 4]) -> io::Result<&'a [u8]> {
    children(b)?
        .into_iter()
        .find(|c| &c[4..8] == kind)
        .ok_or_else(|| invalid("missing required box"))
}
fn replace(b: &[u8], kind: &[u8; 4], replacement: &[u8]) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    for c in children(b)? {
        body.extend_from_slice(if &c[4..8] == kind { replacement } else { c });
    }
    atom(b[4..8].try_into().unwrap(), &body)
}
fn time_scale(b: &[u8]) -> io::Result<u32> {
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
fn duration(b: &[u8], value: u64, track: bool) -> io::Result<Vec<u8>> {
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

#[derive(Clone, Debug)]
struct Sample {
    offset: u64,
    size: u32,
    duration: u32,
    composition_offset: i32,
    sync: bool,
    decode_time: u64,
}
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
}

impl Mp4Builder {
    pub fn new(ftyp: &[u8], moov: &[u8], source: usize) -> io::Result<Self> {
        let mut result = Self {
            ftyp: ftyp.to_vec(),
            mvhd: child(moov, b"mvhd")?.to_vec(),
            tracks: Vec::new(),
            sample_count: 0,
        };
        result.add_source(moov, source)?;
        Ok(result)
    }

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
            let flags = u32_at(tfhd, h)? & 0xffffff;
            let id = u32_at(tfhd, h + 4)?;
            let t = self
                .tracks
                .iter_mut()
                .find(|t| t.source == source && t.id == id)
                .ok_or_else(|| invalid("unknown fragment track"))?;
            let mut p = h + 8;
            let base = if flags & 1 != 0 {
                let n = u64_at(tfhd, p)?;
                p += 8;
                n
            } else {
                // The implicit base for later trafs is the preceding traf's data end.
                // Reject it rather than silently use a wrong source offset.
                if flags & 0x020000 == 0 {
                    return Err(invalid("fragment requires explicit or moof-relative base"));
                }
                moof_offset
            };
            if flags & 2 != 0 {
                if u32_at(tfhd, p)? != 1 {
                    return Err(invalid("multiple sample descriptions unsupported"));
                }
                p += 4;
            }
            let mut defaults = t.defaults;
            for (i, flag) in [8, 16, 32].into_iter().enumerate() {
                if flags & flag != 0 {
                    defaults[i] = u32_at(tfhd, p)?;
                    p += 4;
                }
            }
            let tfdt = child(traf, b"tfdt")?;
            let dh = header(tfdt)?;
            let mut dts = match tfdt.get(dh) {
                Some(0) => u32_at(tfdt, dh + 4)? as u64,
                Some(1) => u64_at(tfdt, dh + 4)?,
                _ => return Err(invalid("invalid tfdt version")),
            };
            let expected = t
                .samples
                .last()
                .map_or(0, |s| s.decode_time + u64::from(s.duration));
            if dts != expected {
                return Err(invalid("discontinuous decode timeline unsupported"));
            }
            let mut cursor = None;
            for trun in children(traf)?.into_iter().filter(|c| &c[4..8] == b"trun") {
                let rh = header(trun)?;
                let version = *trun.get(rh).ok_or_else(|| invalid("short trun"))?;
                if version > 1 {
                    return Err(invalid("invalid trun version"));
                }
                let flags = u32_at(trun, rh)? & 0xffffff;
                let count = u32_at(trun, rh + 4)?;
                self.sample_count = self
                    .sample_count
                    .checked_add(count as usize)
                    .ok_or_else(|| invalid("sample count overflow"))?;
                if self.sample_count > 1_000_000 {
                    return Err(invalid("sample index exceeds memory budget"));
                }
                let mut p = rh + 8;
                if flags & 1 != 0 {
                    cursor = Some(
                        base.checked_add_signed(u32_at(trun, p)? as i32 as i64)
                            .ok_or_else(|| invalid("negative sample offset"))?,
                    );
                    p += 4;
                }
                let mut offset = cursor.ok_or_else(|| invalid("first run has no data offset"))?;
                let first_flags = if flags & 4 != 0 {
                    let f = u32_at(trun, p)?;
                    p += 4;
                    Some(f)
                } else {
                    None
                };
                if first_flags.is_some() && flags & 0x400 != 0 {
                    return Err(invalid("conflicting sample flags"));
                }
                for i in 0..count {
                    let mut values = defaults;
                    if i == 0 {
                        if let Some(f) = first_flags {
                            values[2] = f;
                        }
                    }
                    for (j, flag) in [0x100, 0x200, 0x400].into_iter().enumerate() {
                        if flags & flag != 0 {
                            values[j] = u32_at(trun, p)?;
                            p += 4;
                        }
                    }
                    let cts = if flags & 0x800 != 0 {
                        let v = u32_at(trun, p)?;
                        p += 4;
                        if version == 0 && v > i32::MAX as u32 {
                            return Err(invalid("composition offset overflow"));
                        }
                        v as i32
                    } else {
                        0
                    };
                    if values[0] == 0 || values[1] == 0 {
                        return Err(invalid("zero sample duration or size"));
                    }
                    t.samples.push(Sample {
                        offset,
                        size: values[1],
                        duration: values[0],
                        composition_offset: cts,
                        sync: values[2] & 0x10000 == 0,
                        decode_time: dts,
                    });
                    offset = offset
                        .checked_add(values[1] as u64)
                        .ok_or_else(|| invalid("sample offset overflow"))?;
                    dts = dts
                        .checked_add(values[0] as u64)
                        .ok_or_else(|| invalid("decode time overflow"))?;
                }
                cursor = Some(offset);
            }
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
        let video = self.tracks.iter().position(|t| {
            child(&t.trak, b"mdia").and_then(|m| child(m, b"hdlr"))
                .is_ok_and(|h| h.get(16..20) == Some(b"vide"))
        }).unwrap_or(0);
        let seek_points = self.tracks[video].samples.iter().enumerate()
            .filter(|(_, s)| s.sync)
            .map(|(i, s)| ((s.decode_time as u128 * 1000 / self.tracks[video].timescale as u128) as u64, offsets[video][i]))
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

    fn build_moov(&self, offsets: &[Vec<u64>]) -> io::Result<Vec<u8>> {
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
            movie_duration = movie_duration.max(scaled);
            let mdia = child(&t.trak, b"mdia")?;
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
            let mut tkhd = duration(child(&trak, b"tkhd")?, scaled, true)?;
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

struct Extent {
    output: Range<u64>,
    source: usize,
    source_start: u64,
}
pub struct VirtualMp4 {
    pub prefix: Bytes,
    pub len: u64,
    extents: Vec<Extent>,
    seek_points: Vec<(u64, u64)>,
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
        let start = self.seek_points.get(index.saturating_sub(1))
            .map_or(self.prefix.len() as u64, |(_, offset)| *offset);
        start..start.saturating_add(2 * 1024 * 1024).min(self.len)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn box_of(kind: &[u8; 4], body: Vec<u8>) -> Vec<u8> {
        atom(kind, &body).unwrap()
    }
    fn ints(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }
    fn fixture_moov(edit: bool) -> Vec<u8> {
        let mut media_header = vec![0; 100];
        media_header[12..16].copy_from_slice(&1000u32.to_be_bytes());
        let mvhd = box_of(b"mvhd", media_header.clone());
        let mdhd = box_of(b"mdhd", media_header);
        let mut track_header = vec![0; 84];
        track_header[12..16].copy_from_slice(&1u32.to_be_bytes());
        let tkhd = box_of(b"tkhd", track_header);
        let stsd = box_of(b"stsd", ints(&[0, 0]));
        let stbl = box_of(b"stbl", stsd);
        let minf = box_of(b"minf", stbl);
        let mdia = box_of(b"mdia", [mdhd, minf].concat());
        let edts = if edit {
            box_of(b"edts", box_of(b"elst", ints(&[0, 1, 0, 100, 0x10000])))
        } else {
            vec![]
        };
        let trak = box_of(b"trak", [tkhd, edts, mdia].concat());
        let trex = box_of(b"trex", ints(&[0, 1, 1, 1000, 3, 0]));
        box_of(b"moov", [mvhd, trak, box_of(b"mvex", trex)].concat())
    }
    fn fragment(time: u32, trun: Vec<u8>) -> Vec<u8> {
        box_of(
            b"moof",
            box_of(
                b"traf",
                [
                    box_of(b"tfhd", ints(&[0x20000, 1])),
                    box_of(b"tfdt", ints(&[0, time])),
                    box_of(b"trun", trun),
                ]
                .concat(),
            ),
        )
    }

    #[test]
    fn empty_track_run_returns_error_instead_of_panicking() {
        let mut builder = builder(false);
        let error = builder.add_fragment(0, 100, &fragment(0, vec![])).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    fn builder(edit: bool) -> Mp4Builder {
        Mp4Builder::new(
            &box_of(b"ftyp", b"isom\0\0\0\0isommp41".to_vec()),
            &fixture_moov(edit),
            0,
        )
        .unwrap()
    }

    #[test]
    fn warmup_uses_preceding_keyframe_and_clamps_to_eof() {
        let mut b = builder(false);
        // Explicit flags: sync, non-sync, sync at 0, 1, 2 seconds.
        b.add_fragment(0, 100, &fragment(0, ints(&[0x401, 3, 100, 0, 0x10000, 0])))
            .unwrap();
        let mp4 = b.finish().unwrap();
        let p = mp4.prefix.len() as u64;
        assert_eq!(mp4.seek_warmup_range(0), p..mp4.len);
        assert_eq!(mp4.seek_warmup_range(1), p..mp4.len);
        assert_eq!(mp4.seek_warmup_range(2), p + 6..mp4.len);
        assert_eq!(mp4.seek_warmup_range(u32::MAX), p + 6..mp4.len);
    }

    #[test]
    fn source_tracks_with_same_id_get_unique_output_ids() {
        let mut b = builder(false);
        b.add_source(&fixture_moov(false), 1).unwrap();
        for source in [0, 1] {
            b.add_fragment(source, 100, &fragment(0, ints(&[1, 3, 100])))
                .unwrap();
        }
        let mp4 = b.finish().unwrap();
        let top = box_of(b"root", mp4.prefix[..mp4.prefix.len() - 16].to_vec());
        let moov = child(&top, b"moov").unwrap();
        let tracks: Vec<_> = children(moov)
            .unwrap()
            .into_iter()
            .filter(|c| &c[4..8] == b"trak")
            .collect();
        assert_eq!(u32_at(child(tracks[0], b"tkhd").unwrap(), 20).unwrap(), 1);
        assert_eq!(u32_at(child(tracks[1], b"tkhd").unwrap(), 20).unwrap(), 2);
        assert!(child(moov, b"mvex").is_err());
        assert_eq!(mp4.len - mp4.prefix.len() as u64, 18);
    }

    #[test]
    fn every_partial_range_matches_full_virtual_file() {
        let mut b = builder(false);
        b.add_source(&fixture_moov(false), 1).unwrap();
        for source in [0, 1] {
            b.add_fragment(source, 100, &fragment(0, ints(&[1, 3, 100])))
                .unwrap();
        }
        let mp4 = b.finish().unwrap();
        let sources = [b"abcdefghi", b"123456789"];
        let render = |range| {
            let mut data = Vec::new();
            for part in mp4.read_plan(range).unwrap() {
                match part {
                    ReadPart::Header(r) => data.extend_from_slice(&mp4.prefix[r]),
                    ReadPart::Source { source, range } => data.extend_from_slice(
                        &sources[source][range.start as usize - 200..range.end as usize - 200],
                    ),
                }
            }
            data
        };
        let expected = [mp4.prefix.to_vec(), b"abc123def456ghi789".to_vec()].concat();
        assert_eq!(render(0..mp4.len), expected);
        let p = mp4.prefix.len() as u64;
        // Header/media boundary, arbitrary sample boundaries, EOF and empty ranges.
        for start in p - 5..=mp4.len {
            for end in start..=mp4.len {
                assert_eq!(render(start..end), expected[start as usize..end as usize]);
            }
        }
        assert!(mp4.read_plan(0..mp4.len + 1).is_err());
        assert!(mp4.read_plan(10..9).is_err());
    }

    #[test]
    fn open_ended_edit_duration_is_materialized_without_losing_encoder_delay() {
        let mut b = builder(true);
        b.add_fragment(0, 100, &fragment(0, ints(&[1, 3, 100])))
            .unwrap();
        let mp4 = b.finish().unwrap();
        let top = box_of(b"root", mp4.prefix[..mp4.prefix.len() - 16].to_vec());
        let trak = child(child(&top, b"moov").unwrap(), b"trak").unwrap();
        let elst = child(child(trak, b"edts").unwrap(), b"elst").unwrap();
        assert_eq!(u32_at(elst, 16).unwrap(), 3000);
        assert_eq!(u32_at(elst, 20).unwrap(), 100);
    }

    #[test]
    fn signed_composition_offsets_and_sample_flags_are_preserved() {
        let mut b = builder(false);
        let trun = ints(&[
            0x01000f01,
            2,
            100,
            700,
            5,
            0,
            (-20i32) as u32,
            800,
            6,
            0x10000,
            30,
        ]);
        b.add_fragment(0, 100, &fragment(0, trun)).unwrap();
        let samples = &b.tracks[0].samples;
        assert_eq!((samples[0].offset, samples[1].offset), (200, 205));
        assert_eq!(
            (samples[0].composition_offset, samples[1].composition_offset),
            (-20, 30)
        );
        assert!(samples[0].sync);
        assert!(!samples[1].sync);
        assert_eq!(samples[1].decode_time, 700);
        b.finish().unwrap();
    }

    #[test]
    fn truncated_boxes_and_discontinuous_timeline_fail() {
        assert!(header(&[0, 0, 0, 1, b'm', b'o', b'o', b'f']).is_err());
        assert!(time_scale(&box_of(b"mdhd", vec![])).is_err());
        let mut b = builder(false);
        assert!(
            b.add_fragment(0, 100, &fragment(5000, ints(&[1, 3, 100])))
                .is_err()
        );
        let good = fragment(0, ints(&[1, 3, 100]));
        for end in 0..good.len() {
            assert!(builder(false).add_fragment(0, 100, &good[..end]).is_err());
        }
    }

    #[test]
    fn samples_must_stay_inside_their_own_payload() {
        let moof = fragment(0, ints(&[1, 3, 100]));
        builder(false)
            .add_fragment_in_media(0, 100, &moof, 200..209)
            .unwrap();
        assert!(
            builder(false)
                .add_fragment_in_media(0, 100, &moof, 201..209)
                .is_err()
        );
        assert!(
            builder(false)
                .add_fragment_in_media(0, 100, &moof, 200..208)
                .is_err()
        );
        let mut b = builder(false);
        b.add_fragment(0, 100, &moof).unwrap();
        let mp4 = b.finish().unwrap();
        mp4.validate_sources(&[209]).unwrap();
        assert!(mp4.validate_sources(&[208]).is_err());
        assert!(mp4.validate_sources(&[]).is_err());
    }

    #[actix_web::test]
    async fn http_head_partial_header_and_unsatisfiable_range() {
        use crate::{
            song_cache::{BLOCK_BYTES, RemoteSource, SongCache},
            virtual_mp4_http::serve,
        };
        use actix_web::{
            body::to_bytes,
            http::{Method, StatusCode},
            test::TestRequest,
        };
        use std::sync::Arc;
        let mut b = builder(false);
        b.add_fragment(0, 100, &fragment(0, ints(&[1, 3, 100])))
            .unwrap();
        let mp4 = Arc::new(b.finish().unwrap());
        // Requests in this test must never need a network connection.
        let cache = Arc::new(
            SongCache::new(
                &std::env::temp_dir(),
                reqwest::Client::new(),
                vec![RemoteSource {
                    url: "http://127.0.0.1:1/unreachable".into(),
                    len: 209,
                }],
                BLOCK_BYTES,
            )
            .unwrap(),
        );
        let req = TestRequest::default()
            .method(Method::HEAD)
            .insert_header(("Range", "bytes=0-9"))
            .to_http_request();
        let response = serve(&req, mp4.clone(), cache.clone(), 42);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("Content-Length")
                .unwrap()
                .to_str()
                .unwrap(),
            mp4.len.to_string()
        );
        assert!(to_bytes(response.into_body()).await.unwrap().is_empty());
        let req = TestRequest::default()
            .insert_header(("Range", "bytes=3-12"))
            .to_http_request();
        let response = serve(&req, mp4.clone(), cache.clone(), 42);
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("Content-Range")
                .unwrap()
                .to_str()
                .unwrap(),
            format!("bytes 3-12/{}", mp4.len)
        );
        assert_eq!(
            to_bytes(response.into_body()).await.unwrap(),
            mp4.prefix.slice(3..13)
        );
        let req = TestRequest::default()
            .insert_header(("Range", format!("bytes={}-", mp4.len)))
            .to_http_request();
        assert_eq!(
            serve(&req, mp4.clone(), cache.clone(), 42).status(),
            StatusCode::RANGE_NOT_SATISFIABLE
        );
        let req = TestRequest::default()
            .insert_header(("Range", "bytes=3-12"))
            .insert_header(("If-Range", "\"old-file\""))
            .to_http_request();
        assert_eq!(
            serve(&req, mp4.clone(), cache.clone(), 42).status(),
            StatusCode::OK
        );
        let req = TestRequest::default()
            .insert_header(("If-None-Match", "W/\"ktv-42\""))
            .to_http_request();
        assert_eq!(
            serve(&req, mp4, cache.clone(), 42).status(),
            StatusCode::NOT_MODIFIED
        );
        assert_eq!(cache.stats().0, 0);
    }

    #[actix_web::test]
    async fn seek_retires_a_blocked_response_and_new_range_retries_the_block() {
        use crate::{song_cache::{SongCache, RemoteSource, BLOCK_BYTES}, virtual_mp4_http::serve};
        use actix_web::{body::to_bytes, test::TestRequest};
        use std::{sync::Arc, time::Duration};
        use tokio::{net::TcpListener, io::{AsyncReadExt, AsyncWriteExt}};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/fixture", listener.local_addr().unwrap());
        let (accepted, waiting) = tokio::sync::oneshot::channel();
        let origin = tokio::spawn(async move {
            let mut accepted = Some(accepted);
            for attempt in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut bytes = [0; 512];
                    let n = socket.read(&mut bytes).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&bytes[..n]);
                }
                if attempt == 0 {
                    // Hold the response until cancellation closes this request.
                    accepted.take().unwrap().send(()).unwrap();
                    let mut byte = [0];
                    let closed = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte)).await.unwrap();
                    assert!(matches!(closed, Ok(0) | Err(_)));
                    continue;
                }
                let mut response = b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-208/209\r\nContent-Length: 209\r\nConnection: close\r\n\r\n".to_vec();
                response.extend_from_slice(&[0; 200]);
                response.extend_from_slice(b"abcdefghi");
                socket.write_all(&response).await.unwrap();
            }
        });
        let cache = Arc::new(SongCache::new(&std::env::temp_dir(), reqwest::Client::new(),
            vec![RemoteSource { url, len: 209 }], BLOCK_BYTES).unwrap());
        let mut b = builder(false);
        b.add_fragment(0, 100, &fragment(0, ints(&[1, 3, 100]))).unwrap();
        let mp4 = Arc::new(b.finish().unwrap());
        let request = TestRequest::default().insert_header(("Range", format!("bytes={}-", mp4.prefix.len()))).to_http_request();
        let old = serve(&request, mp4.clone(), cache.clone(), 99);
        let old_read = actix_web::rt::spawn(async move { to_bytes(old.into_body()).await.unwrap() });
        tokio::time::timeout(Duration::from_secs(2), waiting).await.unwrap().unwrap();
        let cutoff = cache.stream_cutoff();
        let new = serve(&request, mp4, cache.clone(), 99);
        cache.retire_streams(cutoff);
        assert!(tokio::time::timeout(Duration::from_secs(2), old_read).await.unwrap().unwrap().is_empty());
        let bytes = tokio::time::timeout(Duration::from_secs(2), to_bytes(new.into_body())).await.unwrap().unwrap();
        assert_eq!(&bytes[..], b"abcdefghi");
        assert_eq!(cache.stats().0, 2);
        origin.await.unwrap();
    }

    #[actix_web::test]
    async fn http_full_and_cross_boundary_ranges_stream_identical_media() {
        use crate::{
            song_cache::{BLOCK_BYTES, RemoteSource, SongCache},
            virtual_mp4_http::serve,
        };
        use actix_web::{body::to_bytes, http::StatusCode, test::TestRequest};
        use std::sync::Arc;
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/fixture", listener.local_addr().unwrap());
        let origin = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut b = [0; 512];
                let n = socket.read(&mut b).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&b[..n]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let mut response = b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-208/209\r\nContent-Length: 209\r\nConnection: close\r\n\r\n".to_vec();
            response.extend_from_slice(&[0; 200]);
            response.extend_from_slice(b"abcdefghi");
            socket.write_all(&response).await.unwrap();
        });
        let cache = Arc::new(
            SongCache::new(
                &std::env::temp_dir(),
                reqwest::Client::new(),
                vec![RemoteSource { url, len: 209 }],
                BLOCK_BYTES,
            )
            .unwrap(),
        );
        let mut b = builder(false);
        b.add_fragment(0, 100, &fragment(0, ints(&[1, 3, 100])))
            .unwrap();
        let mp4 = Arc::new(b.finish().unwrap());
        let full = serve(
            &TestRequest::default().to_http_request(),
            mp4.clone(),
            cache.clone(),
            43,
        );
        let data = to_bytes(full.into_body()).await.unwrap();
        assert_eq!(&data[..mp4.prefix.len()], &mp4.prefix[..]);
        assert_eq!(&data[mp4.prefix.len()..], b"abcdefghi");
        origin.await.unwrap();
        let start = mp4.prefix.len() - 3;
        let req = TestRequest::default()
            .insert_header(("Range", format!("bytes={start}-{}", start + 7)))
            .to_http_request();
        let response = serve(&req, mp4.clone(), cache.clone(), 43);
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            to_bytes(response.into_body()).await.unwrap(),
            data.slice(start..start + 8)
        );
        // Origin has exited: this second request must come entirely from the cache.
        assert_eq!(cache.stats().0, 1);
    }
}
