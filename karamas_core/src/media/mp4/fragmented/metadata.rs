//! 合并初始化段，修改轨道编号、时长和分片时间戳。
use super::{
    AUDIO_TRACK_ID, VIDEO_TRACK_ID,
    boxes::{box_header_len, child, child_range, invalid, make_box, read_u32, read_u64},
    source::SidxInfo,
};
use bytes::{Bytes, BytesMut};
use std::io;

/// 合并音视频初始化信息，统一轨道编号和时长。
pub(super) fn merge_initialization(
    video_ftyp: &Bytes,
    video_moov: &Bytes,
    audio_moov: &Bytes,
    video_sidx: Option<&SidxInfo>,
    audio_sidx: Option<&SidxInfo>,
) -> io::Result<Bytes> {
    let mut mvhd = child(video_moov, b"mvhd")?.to_vec();
    let mut video_trak = child(video_moov, b"trak")?.to_vec();
    let mut audio_trak = child(audio_moov, b"trak")?.to_vec();
    let video_mvex = child(video_moov, b"mvex")?;
    let audio_mvex = child(audio_moov, b"mvex")?;
    let mut video_trex = child(video_mvex, b"trex")?.to_vec();
    let mut audio_trex = child(audio_mvex, b"trex")?.to_vec();

    patch_mvhd_next_track_id(&mut mvhd, 3)?;
    patch_trak_track_id(&mut video_trak, VIDEO_TRACK_ID)?;
    patch_trak_track_id(&mut audio_trak, AUDIO_TRACK_ID)?;
    patch_full_box_u32(&mut video_trex, 12, VIDEO_TRACK_ID)?;
    patch_full_box_u32(&mut audio_trex, 12, AUDIO_TRACK_ID)?;

    // Bilibili 的 fragmented MP4 会把总时长放在 sidx，而 moov 中的
    // duration 通常是 0。我们不会把单轨 sidx 原样拼进双轨文件，但必须
    // 将其中的时长写入初始化段，否则部分 DLNA renderer（Kodi）只会根据
    // 第一个片段推断出约 5 秒的总时长，进而触发错误的自动切歌。
    if let (Some(video_sidx), Some(audio_sidx)) = (video_sidx, audio_sidx) {
        let movie_timescale = mvhd_timescale(&mvhd)?;
        let video_duration =
            scale_duration(video_sidx.duration, video_sidx.timescale, movie_timescale)?;
        let audio_duration =
            scale_duration(audio_sidx.duration, audio_sidx.timescale, movie_timescale)?;
        patch_header_duration(&mut mvhd, video_duration.max(audio_duration))?;
        patch_trak_duration(&mut video_trak, video_sidx, movie_timescale)?;
        patch_trak_duration(&mut audio_trak, audio_sidx, movie_timescale)?;
        log::info!(
            target: "DLNA1080",
            "已写入 DASH 完整时长: video={:.3}s, audio={:.3}s",
            video_sidx.duration as f64 / video_sidx.timescale as f64,
            audio_sidx.duration as f64 / audio_sidx.timescale as f64
        );
    } else {
        log::warn!(target: "DLNA1080", "DASH 初始化段缺少 sidx 时长索引，设备可能暂时显示错误总时长");
    }

    let mvex = make_box(b"mvex", &[&video_trex, &audio_trex])?;
    let moov = make_box(b"moov", &[&mvhd, &video_trak, &audio_trak, &mvex])?;
    let mut init = BytesMut::with_capacity(video_ftyp.len() + moov.len());
    init.extend_from_slice(video_ftyp);
    init.extend_from_slice(&moov);
    Ok(init.freeze())
}

/// 更新分片的轨道编号和序号。
pub(super) fn patch_moof(moof: &Bytes, track_id: u32, sequence: u32) -> io::Result<Bytes> {
    let mut out = moof.to_vec();
    let mfhd = child_range(&out, 0..out.len(), b"mfhd")?;
    patch_at(&mut out, mfhd.start + 12, sequence)?;
    let traf = child_range(&out, 0..out.len(), b"traf")?;
    let tfhd = child_range(&out, traf, b"tfhd")?;
    patch_at(&mut out, tfhd.start + 12, track_id)?;
    Ok(Bytes::from(out))
}

/// 按轨道时间刻度延后分片的解码时间。
pub(crate) fn delay_fragment(data: &[u8], timescale: u32, milliseconds: u32) -> io::Result<Bytes> {
    if milliseconds == 0 {
        return Ok(Bytes::copy_from_slice(data));
    }
    if timescale == 0 {
        return Err(invalid("zero track timescale"));
    }
    let ticks = (u64::from(milliseconds) * u64::from(timescale)).div_ceil(1000);
    let mut output = data.to_vec();
    let mut found = false;
    for moof in crate::media::mp4::boxes::boxes(data)? {
        if &moof[4..8] != b"moof" {
            continue;
        }
        for traf in crate::media::mp4::boxes::children(moof)? {
            if &traf[4..8] != b"traf" {
                continue;
            }
            let tfdt = crate::media::mp4::boxes::child(traf, b"tfdt")?;
            let h = crate::media::mp4::boxes::header(tfdt)?;
            let offset = tfdt.as_ptr() as usize - data.as_ptr() as usize + h + 4;
            match tfdt.get(h) {
                Some(0) => {
                    let time = read_u32(data, offset)?
                        .checked_add(
                            u32::try_from(ticks).map_err(|_| invalid("timestamp overflow"))?,
                        )
                        .ok_or_else(|| invalid("timestamp overflow"))?;
                    patch_at(&mut output, offset, time)?;
                }
                Some(1) => {
                    let time = read_u64(data, offset)?
                        .checked_add(ticks)
                        .ok_or_else(|| invalid("timestamp overflow"))?;
                    patch_u64(&mut output, offset, time)?;
                }
                _ => return Err(invalid("invalid tfdt version")),
            }
            found = true;
        }
    }
    if !found {
        return Err(invalid("missing fragment timestamps"));
    }
    Ok(Bytes::from(output))
}

/// 读取分片的起始解码时间。
pub(super) fn fragment_decode_time(moof: &[u8]) -> io::Result<u64> {
    let traf = child_range(moof, 0..moof.len(), b"traf")?;
    let tfdt = child_range(moof, traf, b"tfdt")?;
    let version = *moof
        .get(tfdt.start + 8)
        .ok_or_else(|| invalid("short tfdt"))?;
    if version == 1 {
        read_u64(moof, tfdt.start + 12)
    } else {
        Ok(read_u32(moof, tfdt.start + 12)? as u64)
    }
}

/// 读取轨道的时间刻度。
pub(super) fn track_timescale(moov: &[u8]) -> io::Result<u32> {
    let trak = child_range(moov, 0..moov.len(), b"trak")?;
    let mdia = child_range(moov, trak, b"mdia")?;
    let mdhd = child_range(moov, mdia, b"mdhd")?;
    let version = *moov
        .get(mdhd.start + 8)
        .ok_or_else(|| invalid("short mdhd"))?;
    let offset = if version == 1 {
        mdhd.start + 28
    } else {
        mdhd.start + 20
    };
    let timescale = read_u32(moov, offset)?;
    if timescale == 0 {
        Err(invalid("zero track timescale"))
    } else {
        Ok(timescale)
    }
}

/// 读取影片的时间刻度。
fn mvhd_timescale(mvhd: &[u8]) -> io::Result<u32> {
    let header = box_header_len(mvhd)?;
    let version = *mvhd.get(header).ok_or_else(|| invalid("short mvhd"))?;
    let offset = if version == 1 {
        header + 20
    } else {
        header + 12
    };
    let timescale = read_u32(mvhd, offset)?;
    if timescale == 0 {
        Err(invalid("zero movie timescale"))
    } else {
        Ok(timescale)
    }
}

/// 根据分片索引更新轨道时长。
fn patch_trak_duration(trak: &mut [u8], sidx: &SidxInfo, movie_timescale: u32) -> io::Result<()> {
    let tkhd = child_range(trak, 0..trak.len(), b"tkhd")?;
    let mdia = child_range(trak, 0..trak.len(), b"mdia")?;
    let mdhd = child_range(trak, mdia, b"mdhd")?;
    let media_timescale = {
        let header = box_header_len(&trak[mdhd.clone()])?;
        let version = *trak
            .get(mdhd.start + header)
            .ok_or_else(|| invalid("short mdhd"))?;
        read_u32(
            trak,
            mdhd.start
                + if version == 1 {
                    header + 20
                } else {
                    header + 12
                },
        )?
    };
    if media_timescale == 0 {
        return Err(invalid("zero media timescale"));
    }
    let media_duration = scale_duration(sidx.duration, sidx.timescale, media_timescale)?;
    patch_header_duration(&mut trak[mdhd], media_duration)?;
    let movie_duration = scale_duration(sidx.duration, sidx.timescale, movie_timescale)?;
    patch_tkhd_duration(&mut trak[tkhd], movie_duration)?;
    Ok(())
}

/// 将时长换算到目标时间刻度。
fn scale_duration(duration: u64, source_timescale: u32, target_timescale: u32) -> io::Result<u64> {
    if source_timescale == 0 || target_timescale == 0 {
        return Err(invalid("zero duration timescale"));
    }
    u64::try_from((duration as u128) * (target_timescale as u128) / (source_timescale as u128))
        .map_err(|_| invalid("duration overflow"))
}

/// 更新 mvhd 或 mdhd 中的时长。
fn patch_header_duration(atom: &mut [u8], duration: u64) -> io::Result<()> {
    let header = box_header_len(atom)?;
    let version = *atom.get(header).ok_or_else(|| invalid("short full box"))?;
    let offset = if version == 1 {
        header + 24
    } else {
        header + 16
    };
    if version == 1 {
        patch_u64(atom, offset, duration)
    } else {
        patch_at(
            atom,
            offset,
            u32::try_from(duration).map_err(|_| invalid("v0 duration overflow"))?,
        )
    }
}

/// 更新 tkhd 中的轨道时长。
fn patch_tkhd_duration(atom: &mut [u8], duration: u64) -> io::Result<()> {
    let header = box_header_len(atom)?;
    let version = *atom.get(header).ok_or_else(|| invalid("short tkhd"))?;
    let offset = if version == 1 {
        header + 28
    } else {
        header + 20
    };
    if version == 1 {
        patch_u64(atom, offset, duration)
    } else {
        patch_at(
            atom,
            offset,
            u32::try_from(duration).map_err(|_| invalid("v0 duration overflow"))?,
        )
    }
}

/// 更新轨道头中的轨道编号。
fn patch_trak_track_id(trak: &mut [u8], track_id: u32) -> io::Result<()> {
    let tkhd = child_range(trak, 0..trak.len(), b"tkhd")?;
    let version = *trak
        .get(tkhd.start + 8)
        .ok_or_else(|| invalid("short tkhd"))?;
    let offset = if version == 1 {
        tkhd.start + 28
    } else {
        tkhd.start + 20
    };
    patch_at(trak, offset, track_id)
}

/// 更新影片头中的下一个可用轨道编号。
fn patch_mvhd_next_track_id(mvhd: &mut [u8], next_track_id: u32) -> io::Result<()> {
    if mvhd.len() < 12 {
        return Err(invalid("short mvhd"));
    }
    patch_at(mvhd, mvhd.len() - 4, next_track_id)
}

/// 更新 box 中指定偏移的 u32 字段。
fn patch_full_box_u32(atom: &mut [u8], offset: usize, value: u32) -> io::Result<()> {
    patch_at(atom, offset, value)
}

/// 在指定偏移写入大端 u32。
fn patch_at(data: &mut [u8], offset: usize, value: u32) -> io::Result<()> {
    let dst = data
        .get_mut(offset..offset + 4)
        .ok_or_else(|| invalid("box patch out of bounds"))?;
    dst.copy_from_slice(&value.to_be_bytes());
    Ok(())
}

/// 在指定偏移写入大端 u64。
fn patch_u64(data: &mut [u8], offset: usize, value: u64) -> io::Result<()> {
    let dst = data
        .get_mut(offset..offset + 8)
        .ok_or_else(|| invalid("box patch out of bounds"))?;
    dst.copy_from_slice(&value.to_be_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_fragment_ids_without_changing_size() {
        let mfhd = make_box(b"mfhd", &[&[0, 0, 0, 0, 0, 0, 0, 9]]).unwrap();
        let tfhd = make_box(b"tfhd", &[&[0, 0, 0, 0, 0, 0, 0, 1]]).unwrap();
        let traf = make_box(b"traf", &[&tfhd]).unwrap();
        let moof = Bytes::from(make_box(b"moof", &[&mfhd, &traf]).unwrap());
        let patched = patch_moof(&moof, 2, 7).unwrap();
        assert_eq!(patched.len(), moof.len());
        let mfhd = child_range(&patched, 0..patched.len(), b"mfhd").unwrap();
        let traf = child_range(&patched, 0..patched.len(), b"traf").unwrap();
        let tfhd = child_range(&patched, traf, b"tfhd").unwrap();
        assert_eq!(read_u32(&patched, mfhd.start + 12).unwrap(), 7);
        assert_eq!(read_u32(&patched, tfhd.start + 12).unwrap(), 2);
    }

    #[test]
    fn patches_v0_movie_header_duration() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0, 0, 0, 0]); // version + flags
        body.extend_from_slice(&0u32.to_be_bytes()); // creation time
        body.extend_from_slice(&0u32.to_be_bytes()); // modification time
        body.extend_from_slice(&1_000u32.to_be_bytes()); // timescale
        body.extend_from_slice(&0u32.to_be_bytes()); // duration
        body.resize(100, 0);
        let mut mvhd = make_box(b"mvhd", &[&body]).unwrap();

        assert_eq!(mvhd_timescale(&mvhd).unwrap(), 1_000);
        patch_header_duration(&mut mvhd, 285_000).unwrap();
        assert_eq!(read_u32(&mvhd, 24).unwrap(), 285_000);
    }

    #[test]
    fn patches_v0_track_header_duration() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0, 0, 0, 0]); // version + flags
        body.extend_from_slice(&0u32.to_be_bytes()); // creation time
        body.extend_from_slice(&0u32.to_be_bytes()); // modification time
        body.extend_from_slice(&1u32.to_be_bytes()); // track id
        body.extend_from_slice(&0u32.to_be_bytes()); // reserved
        body.extend_from_slice(&0u32.to_be_bytes()); // duration
        body.resize(100, 0);
        let mut tkhd = make_box(b"tkhd", &[&body]).unwrap();

        patch_tkhd_duration(&mut tkhd, 285_000).unwrap();
        assert_eq!(read_u32(&tkhd, 28).unwrap(), 285_000);
    }
}
