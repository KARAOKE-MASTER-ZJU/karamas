//! MP4 box 的边界校验和读取，供索引与 CENC 共用。
use std::io;

/// 构建 MP4 数据无效的错误。
pub(crate) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
/// 从指定偏移读取大端 u32。
pub(crate) fn u32_at(b: &[u8], p: usize) -> io::Result<u32> {
    Ok(u32::from_be_bytes(
        b.get(p..p + 4)
            .ok_or_else(|| invalid("short u32"))?
            .try_into()
            .unwrap(),
    ))
}
/// 从指定偏移读取大端 u64。
pub(crate) fn u64_at(b: &[u8], p: usize) -> io::Result<u64> {
    Ok(u64::from_be_bytes(
        b.get(p..p + 8)
            .ok_or_else(|| invalid("short u64"))?
            .try_into()
            .unwrap(),
    ))
}
/// 将数据封装为指定类型的 MP4 box。
pub(crate) fn atom(kind: &[u8; 4], body: &[u8]) -> io::Result<Vec<u8>> {
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
/// 检查 MP4 box 头部并返回头部长度。
pub(crate) fn header(b: &[u8]) -> io::Result<usize> {
    if b.len() < 8 {
        return Err(invalid("short box"));
    }
    let size = if u32_at(b, 0)? == 1 { 16 } else { 8 };
    if b.len() < size {
        return Err(invalid("short extended box header"));
    }
    Ok(size)
}
/// 解析并校验容器内的直接子 box。
pub(crate) fn children(b: &[u8]) -> io::Result<Vec<&[u8]>> {
    let p = header(b)?;
    let declared = match u32_at(b, 0)? {
        0 => b.len() as u64,
        1 => u64_at(b, 8)?,
        n => n as u64,
    };
    if declared != b.len() as u64 {
        return Err(invalid("container size mismatch"));
    }
    boxes(&b[p..])
}

/// 解析完整文件或已定位的 box 序列，不假设外层还有容器头。
pub(crate) fn boxes(b: &[u8]) -> io::Result<Vec<&[u8]>> {
    let mut p = 0;
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
/// 获取指定类型的直接子 box。
pub(crate) fn child<'a>(b: &'a [u8], kind: &[u8; 4]) -> io::Result<&'a [u8]> {
    children(b)?
        .into_iter()
        .find(|c| &c[4..8] == kind)
        .ok_or_else(|| invalid("missing required box"))
}
