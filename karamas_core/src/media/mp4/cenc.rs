//! 单轨 CENC/AES-CTR 分片解密，保持样本布局和时间戳。
use super::{
    boxes::{boxes, child, children, header, invalid, u32_at},
    indexed::fragment_samples,
};
use bytes::Bytes;
use ctr::cipher::{KeyIvInit, StreamCipher};
use std::{collections::HashMap, io};

pub(crate) type Keys = HashMap<[u8; 16], [u8; 16]>;

#[derive(Clone, Copy)]
struct Encryption {
    kid: [u8; 16],
    iv_size: usize,
    protected: bool,
}

pub(crate) struct Track {
    pub initialization: Bytes,
    pub timescale: u32,
    id: u32,
    defaults: [u32; 3],
    encryption: Option<Encryption>,
}

fn body(data: &[u8]) -> io::Result<&[u8]> {
    Ok(&data[header(data)?..])
}
fn offset(all: &[u8], part: &[u8]) -> usize {
    part.as_ptr() as usize - all.as_ptr() as usize
}
fn take_u16(data: &[u8], position: &mut usize) -> io::Result<u16> {
    let value = u16::from_be_bytes(
        data.get(*position..*position + 2)
            .ok_or_else(|| invalid("CENC 字段不完整"))?
            .try_into()
            .unwrap(),
    );
    *position += 2;
    Ok(value)
}
fn take_u32(data: &[u8], position: &mut usize) -> io::Result<u32> {
    let value = u32_at(data, *position)?;
    *position += 4;
    Ok(value)
}
fn info(data: &[u8]) -> io::Result<Encryption> {
    if data.len() < 20
        || data[1] != 0
        || data[2] > 1
        || (data[2] == 1 && !matches!(data[3], 8 | 16))
    {
        return Err(invalid("不支持的 CENC 加密参数"));
    }
    Ok(Encryption {
        kid: data[4..20].try_into().unwrap(),
        iv_size: usize::from(data[3]),
        protected: data[2] == 1,
    })
}

impl Track {
    pub fn new(initialization: &[u8]) -> io::Result<Self> {
        let root = boxes(initialization)?;
        let moov = root
            .iter()
            .find(|b| &b[4..8] == b"moov")
            .ok_or_else(|| invalid("MP4 缺少 moov"))?;
        let tracks: Vec<_> = children(moov)?
            .into_iter()
            .filter(|b| &b[4..8] == b"trak")
            .collect();
        if tracks.len() != 1 {
            return Err(invalid("初始化段必须只包含一条轨道"));
        }
        let trak = tracks[0];
        let mdia = child(trak, b"mdia")?;
        let stsd = child(child(child(mdia, b"minf")?, b"stbl")?, b"stsd")?;
        let descriptions = body(stsd)?;
        if u32_at(descriptions, 4)? != 1 {
            return Err(invalid("直播轨道含多个样本描述"));
        }
        let entries = boxes(
            descriptions
                .get(8..)
                .ok_or_else(|| invalid("样本描述不完整"))?,
        )?;
        if entries.len() != 1 {
            return Err(invalid("样本描述数量不匹配"));
        }
        let entry = entries[0];
        let mut clear = initialization.to_vec();
        let mut encryption = None;
        let video = matches!(&entry[4..8], b"encv" | b"avc1");
        if !video && !matches!(&entry[4..8], b"enca" | b"mp4a") {
            return Err(invalid("不支持的直播编码格式"));
        }
        let extra = body(entry)?
            .get(if video { 78 } else { 28 }..)
            .ok_or_else(|| invalid("样本描述不完整"))?;
        let mut parts = boxes(extra)?;
        if matches!(&entry[4..8], b"encv" | b"enca") {
            let sinf = parts
                .iter()
                .copied()
                .find(|b| &b[4..8] == b"sinf")
                .ok_or_else(|| invalid("MP4 缺少加密描述"))?;
            if body(child(sinf, b"schm")?)?.get(4..8) != Some(b"cenc") {
                return Err(invalid("当前只支持 CENC/AES-CTR"));
            }
            let tenc = body(child(child(sinf, b"schi")?, b"tenc")?)?;
            if tenc.first().is_none_or(|v| *v > 1) {
                return Err(invalid("不支持的 tenc 版本"));
            }
            encryption = Some(info(tenc.get(4..).ok_or_else(|| invalid("tenc 不完整"))?)?);
            let original = body(child(sinf, b"frma")?)?;
            if original != if video { b"avc1" } else { b"mp4a" } {
                return Err(invalid("加密描述与媒体格式不匹配"));
            }
            let p = offset(initialization, entry);
            clear[p + 4..p + 8].copy_from_slice(original);
        }
        // 编码配置放在首个子 box，兼容项目现有的 mp4 读取器；整体大小保持不变。
        let config = if video { b"avcC" } else { b"esds" };
        if !parts.iter().any(|b| &b[4..8] == config) {
            return Err(invalid("缺少编码配置"));
        }
        parts.sort_by_key(|b| &b[4..8] != config);
        let mut p = offset(initialization, extra);
        for part in parts {
            clear[p..p + part.len()].copy_from_slice(part);
            // 用同大小的 free 替代加密描述，已有分片的样本偏移无需重写。
            if &part[4..8] == b"sinf" {
                clear[p + 4..p + 8].copy_from_slice(b"free");
            }
            p += part.len();
        }
        for pssh in root
            .iter()
            .copied()
            .chain(children(moov)?)
            .filter(|b| &b[4..8] == b"pssh")
        {
            clear[offset(initialization, pssh) + 4..offset(initialization, pssh) + 8]
                .copy_from_slice(b"free");
        }
        let reader = mp4::Mp4Reader::read_header(std::io::Cursor::new(&clear), clear.len() as u64)
            .map_err(|_| invalid("MP4 初始化信息无效"))?;
        let track = reader
            .tracks()
            .values()
            .next()
            .ok_or_else(|| invalid("MP4 缺少轨道"))?;
        let id = track.track_id();
        let timescale = track.timescale();
        if timescale == 0 {
            return Err(invalid("MP4 时间刻度不能为零"));
        }
        let trex = &reader
            .moov
            .mvex
            .as_ref()
            .ok_or_else(|| invalid("缺少分片默认参数"))?
            .trex;
        if trex.track_id != id || trex.default_sample_description_index != 1 {
            return Err(invalid("分片默认参数与轨道不匹配"));
        }
        Ok(Self {
            initialization: clear.into(),
            timescale,
            id,
            defaults: [
                trex.default_sample_duration,
                trex.default_sample_size,
                trex.default_sample_flags,
            ],
            encryption,
        })
    }

    pub fn decrypt(&self, data: &[u8], keys: &Keys) -> io::Result<Bytes> {
        let Some(default) = self.encryption else {
            return Ok(Bytes::copy_from_slice(data));
        };
        let top = boxes(data)?;
        let media: Vec<_> = top
            .iter()
            .filter(|b| &b[4..8] == b"mdat")
            .map(|b| Ok(offset(data, b) + header(b)?..offset(data, b) + b.len()))
            .collect::<io::Result<_>>()?;
        let mut output = data.to_vec();
        let mut count = 0;
        let mut used = Vec::new();
        for moof in top.into_iter().filter(|b| &b[4..8] == b"moof") {
            for traf in children(moof)?.into_iter().filter(|b| &b[4..8] == b"traf") {
                if u32_at(body(child(traf, b"tfhd")?)?, 4)? != self.id {
                    return Err(invalid("分片轨道与初始化段不匹配"));
                }
                let samples = fragment_samples(traf, offset(data, moof) as u64, self.defaults)?;
                let settings = sample_groups(traf, default, samples.len())?;
                let senc = if settings.iter().any(|info| info.protected) {
                    let senc = body(child(traf, b"senc")?)?;
                    if u32_at(senc, 0)? & !2 != 0 || u32_at(senc, 4)? as usize != samples.len() {
                        return Err(invalid("CENC 样本描述无效"));
                    }
                    Some(senc)
                } else {
                    None
                };
                let mut cursor = 8;
                for (sample, setting) in samples.iter().zip(settings) {
                    let range = sample.range();
                    let start =
                        usize::try_from(range.start).map_err(|_| invalid("样本偏移溢出"))?;
                    let end = usize::try_from(range.end).map_err(|_| invalid("样本偏移溢出"))?;
                    if !media.iter().any(|r| start >= r.start && end <= r.end) {
                        return Err(invalid("CENC 样本超出 mdat"));
                    }
                    used.push(start..end);
                    if setting.protected {
                        decrypt_sample(
                            &mut output[start..end],
                            senc.unwrap(),
                            &mut cursor,
                            setting,
                            keys,
                        )?;
                    }
                }
                if senc.is_some_and(|senc| cursor != senc.len()) {
                    return Err(invalid("CENC 加密描述长度不匹配"));
                }
                count += samples.len();
                for item in children(traf)? {
                    if matches!(&item[4..8], b"senc" | b"saiz" | b"saio")
                        || (matches!(&item[4..8], b"sgpd" | b"sbgp")
                            && body(item)?.get(4..8) == Some(b"seig"))
                    {
                        let p = offset(data, item);
                        output[p + 4..p + 8].copy_from_slice(b"free");
                    }
                }
            }
        }
        if count == 0 {
            return Err(invalid("分片没有媒体样本"));
        }
        used.sort_unstable_by_key(|range| range.start);
        if used.windows(2).any(|pair| pair[0].end > pair[1].start) {
            return Err(invalid("CENC 样本数据相互重叠"));
        }
        Ok(output.into())
    }
}

fn sample_groups(traf: &[u8], default: Encryption, count: usize) -> io::Result<Vec<Encryption>> {
    let mut local = None;
    let mut mapping = None;
    for atom in children(traf)? {
        if !matches!(&atom[4..8], b"sgpd" | b"sbgp") {
            continue;
        }
        let b = body(atom)?;
        if b.get(4..8) != Some(b"seig") {
            continue;
        }
        if &atom[4..8] == b"sgpd" {
            // 支持每个分片一个 seig 描述；sbgp 仍逐段校验，不能把分组盲目套给所有样本。
            if b.first() != Some(&1) || u32_at(b, 8)? != 20 || u32_at(b, 12)? != 1 {
                return Err(invalid("不支持的 CENC 密钥分组"));
            }
            local = Some(info(&b[16..])?);
        } else {
            mapping = Some(b);
        }
    }
    let mut result = vec![default; count];
    if let Some(b) = mapping {
        let mut p = match b.first() {
            Some(0) => 8,
            Some(1) => 12,
            _ => return Err(invalid("sbgp 版本无效")),
        };
        let entries = take_u32(b, &mut p)?;
        let mut sample = 0usize;
        for _ in 0..entries {
            let n = take_u32(b, &mut p)? as usize;
            let index = take_u32(b, &mut p)?;
            let setting = match index {
                0 => default,
                // ISO 14496-12：1 起为轨道级索引，0x10001 起才引用分片内的分组。
                0x10001 => local.ok_or_else(|| invalid("缺少 seig 描述"))?,
                1..=0x10000 => return Err(invalid("当前不支持轨道级 CENC 密钥分组")),
                _ => return Err(invalid("不支持的 CENC 分组索引")),
            };
            let end = sample
                .checked_add(n)
                .filter(|n| *n <= count)
                .ok_or_else(|| invalid("sbgp 样本数无效"))?;
            result[sample..end].fill(setting);
            sample = end;
        }
        if sample != count || p != b.len() {
            return Err(invalid("sbgp 未覆盖完整分片"));
        }
    } else if local.is_some() {
        return Err(invalid("seig 缺少样本分组映射"));
    }
    Ok(result)
}

fn decrypt_sample(
    payload: &mut [u8],
    senc: &[u8],
    cursor: &mut usize,
    info: Encryption,
    keys: &Keys,
) -> io::Result<()> {
    let key = keys
        .get(&info.kid)
        .ok_or_else(|| invalid("直播密钥未覆盖当前分片，请重新播放"))?;
    let mut iv = [0; 16];
    iv[..info.iv_size].copy_from_slice(
        senc.get(*cursor..*cursor + info.iv_size)
            .ok_or_else(|| invalid("CENC IV 不完整"))?,
    );
    *cursor += info.iv_size;
    // 每个样本独立使用 IV；8 字节 IV 的低 64 位从零计数。
    let mut cipher = ctr::Ctr64BE::<aes::Aes128>::new(key.into(), (&iv).into());
    if u32_at(senc, 0)? & 2 == 0 {
        return cipher
            .try_apply_keystream(payload)
            .map_err(|_| invalid("CENC 计数器溢出"));
    }
    let count = take_u16(senc, cursor)?;
    if count == 0 {
        return cipher
            .try_apply_keystream(payload)
            .map_err(|_| invalid("CENC 计数器溢出"));
    }
    // 明文区不消耗密钥流，加密区之间也不能重新初始化 CTR。
    let mut position = 0usize;
    for _ in 0..count {
        let clear = usize::from(take_u16(senc, cursor)?);
        let encrypted = take_u32(senc, cursor)? as usize;
        let start = position
            .checked_add(clear)
            .ok_or_else(|| invalid("CENC 子样本溢出"))?;
        position = start
            .checked_add(encrypted)
            .ok_or_else(|| invalid("CENC 子样本溢出"))?;
        cipher
            .try_apply_keystream(
                payload
                    .get_mut(start..position)
                    .ok_or_else(|| invalid("CENC 子样本越界"))?,
            )
            .map_err(|_| invalid("CENC 计数器溢出"))?;
    }
    if position != payload.len() {
        return Err(invalid("CENC 子样本未覆盖完整样本"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex_key(text: &str) -> Result<[u8; 16], std::num::ParseIntError> {
        let mut output = [0; 16];
        for (i, byte) in output.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)?;
        }
        Ok(output)
    }

    // CENC 的明文字节不消耗 CTR 密钥流；跨子样本保持连续，来自 NIST SP 800-38A F.5.1。
    #[test]
    fn cenc_clear_gaps_do_not_consume_the_ctr_keystream() {
        let kid = [0; 16];
        let key = hex_key("2b7e151628aed2a6abf7158809cf4f3c").unwrap();
        let iv = hex_key("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff").unwrap();
        let mut payload = b"clear".to_vec();
        payload.extend_from_slice(&hex_key("874d6191b620e3261bef6864990db6ce").unwrap());
        payload.extend_from_slice(b"gap");
        payload.extend_from_slice(&hex_key("9806f66b7970fdff8617187bb9fffdff").unwrap());
        let mut senc = vec![0, 0, 0, 2, 0, 0, 0, 1];
        senc.extend_from_slice(&iv);
        senc.extend_from_slice(&[0, 2, 0, 5, 0, 0, 0, 16, 0, 3, 0, 0, 0, 16]);
        decrypt_sample(
            &mut payload,
            &senc,
            &mut 8,
            Encryption {
                kid,
                iv_size: 16,
                protected: true,
            },
            &Keys::from([(kid, key)]),
        )
        .unwrap();
        let mut expected = b"clear".to_vec();
        expected.extend_from_slice(&hex_key("6bc1bee22e409f96e93d7e117393172a").unwrap());
        expected.extend_from_slice(b"gap");
        expected.extend_from_slice(&hex_key("ae2d8a571e03ac9c9eb76fac45af8e51").unwrap());
        assert_eq!(payload, expected);
    }
}
