//! 分片样本的偏移、时长和标志解析，供索引与 CENC 共用。
use super::boxes::{child, children, header, invalid, u32_at, u64_at};
use std::{io, ops::Range};

#[derive(Clone, Debug)]
pub(crate) struct Sample {
    pub(super) offset: u64,
    pub(super) size: u32,
    pub(super) duration: u32,
    pub(super) composition_offset: i32,
    pub(super) sync: bool,
    pub(super) decode_time: u64,
}
impl Sample {
    pub(crate) fn range(&self) -> Range<u64> {
        self.offset..self.offset + u64::from(self.size)
    }
}

/// 解析一个 traf 的样本位置；连续时间轴属于点播索引策略，由调用方校验。
pub(crate) fn fragment_samples(
    traf: &[u8],
    moof_offset: u64,
    defaults: [u32; 3],
) -> io::Result<Vec<Sample>> {
    let tfhd = child(traf, b"tfhd")?;
    let h = header(tfhd)?;
    let flags = u32_at(tfhd, h)? & 0xffffff;
    let mut samples = Vec::new();
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
    let mut defaults = defaults;
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
    let mut cursor = None;
    for trun in children(traf)?.into_iter().filter(|c| &c[4..8] == b"trun") {
        let rh = header(trun)?;
        let version = *trun.get(rh).ok_or_else(|| invalid("short trun"))?;
        if version > 1 {
            return Err(invalid("invalid trun version"));
        }
        let flags = u32_at(trun, rh)? & 0xffffff;
        let count = u32_at(trun, rh + 4)?;
        if samples.len().saturating_add(count as usize) > 1_000_000 {
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
            samples.push(Sample {
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
    if samples.is_empty() {
        return Err(invalid("fragment has no samples"));
    }
    Ok(samples)
}
