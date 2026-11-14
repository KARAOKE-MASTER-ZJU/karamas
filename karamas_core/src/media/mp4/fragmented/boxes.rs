//! 流式 fMP4 的 box 读取和构造；保留对延伸到 EOF 的 box 的拒绝规则。
use std::{io, ops::Range};

/// 获取指定类型的直接子 box。
pub(super) fn child<'a>(parent: &'a [u8], wanted: &[u8; 4]) -> io::Result<&'a [u8]> {
    let range = child_range(parent, 0..parent.len(), wanted)?;
    Ok(&parent[range])
}

/// 查找指定类型的直接子 box，返回其字节范围。
pub(super) fn child_range(
    data: &[u8],
    parent: Range<usize>,
    wanted: &[u8; 4],
) -> io::Result<Range<usize>> {
    let parent_header = box_header_len(&data[parent.clone()])?;
    let mut offset = parent.start + parent_header;
    while offset + 8 <= parent.end {
        let size = box_size_prefix(&data[offset..parent.end])?;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| invalid("box size overflow"))?;
        if end > parent.end {
            return Err(invalid("child box exceeds parent"));
        }
        if data.get(offset + 4..offset + 8) == Some(wanted.as_slice()) {
            return Ok(offset..end);
        }
        offset = end;
    }
    Err(invalid(format!(
        "missing {} box",
        String::from_utf8_lossy(wanted)
    )))
}

/// 将给定内容封装为指定类型的 MP4 box。
pub(super) fn make_box(kind: &[u8; 4], children: &[&[u8]]) -> io::Result<Vec<u8>> {
    let payload: usize = children.iter().map(|c| c.len()).sum();
    let size = 8usize
        .checked_add(payload)
        .ok_or_else(|| invalid("box size overflow"))?;
    let size32 = u32::try_from(size).map_err(|_| invalid("initialization box too large"))?;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&size32.to_be_bytes());
    out.extend_from_slice(kind);
    for child in children {
        out.extend_from_slice(child);
    }
    Ok(out)
}

/// 读取 MP4 box 的类型。
pub(super) fn atom_type(data: &[u8]) -> io::Result<&[u8]> {
    data.get(4..8).ok_or_else(|| invalid("short box header"))
}

/// 读取 MP4 box 的头部长度。
pub(super) fn box_header_len(data: &[u8]) -> io::Result<usize> {
    Ok(if read_u32(data, 0)? == 1 { 16 } else { 8 })
}

/// 读取 MP4 box 头部声明的大小。
pub(super) fn box_size_prefix(data: &[u8]) -> io::Result<usize> {
    let size32 = read_u32(data, 0)?;
    if size32 == 0 {
        return Err(invalid("box extending to EOF unsupported"));
    }
    if size32 == 1 {
        usize::try_from(read_u64(data, 8)?).map_err(|_| invalid("box too large"))
    } else {
        Ok(size32 as usize)
    }
}

/// 从指定偏移读取大端 u16。
pub(super) fn read_u16(data: &[u8], offset: usize) -> io::Result<u16> {
    let b: [u8; 2] = data
        .get(offset..offset + 2)
        .ok_or_else(|| invalid("short u16"))?
        .try_into()
        .unwrap();
    Ok(u16::from_be_bytes(b))
}

/// 从指定偏移读取大端 u32。
pub(super) fn read_u32(data: &[u8], offset: usize) -> io::Result<u32> {
    let b: [u8; 4] = data
        .get(offset..offset + 4)
        .ok_or_else(|| invalid("short u32"))?
        .try_into()
        .unwrap();
    Ok(u32::from_be_bytes(b))
}

/// 从指定偏移读取大端 u64。
pub(super) fn read_u64(data: &[u8], offset: usize) -> io::Result<u64> {
    let b: [u8; 8] = data
        .get(offset..offset + 8)
        .ok_or_else(|| invalid("short u64"))?
        .try_into()
        .unwrap();
    Ok(u64::from_be_bytes(b))
}

/// 构建 MP4 数据无效的错误。
pub(super) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
