//! MMF 使用的动态 SegmentTemplate/SegmentTimeline 描述，不处理媒体样本。
use crate::{
    cast::Quality,
    sources::live::{hex_key, invalid, media_url, target_height},
};
use roxmltree::{Document, Node};
use std::{collections::BTreeMap, io, time::Instant};
use url::Url;

pub(super) struct Segment {
    pub url: Url,
    pub duration: u64,
}
pub(super) struct Representation {
    pub init: Url,
    pub timescale: u32,
    pub height: u32,
    pub bandwidth: u64,
    pub codecs: String,
    pub kid: Option<[u8; 16]>,
    pub segments: BTreeMap<u64, Segment>,
}
pub(super) struct Manifest {
    pub video: Representation,
    pub audio: Representation,
    pub period: String,
    pub refreshed: Instant,
}
pub(super) fn parse(xml: &[u8], effective: &Url, quality: Quality) -> io::Result<Manifest> {
    let text = std::str::from_utf8(xml).map_err(|_| invalid("DASH 清单不是 UTF-8"))?;
    let document = Document::parse(text).map_err(|_| invalid("DASH 清单格式无效"))?;
    let root = document.root_element();
    if !root.has_tag_name("MPD") || root.attribute("type") != Some("dynamic") {
        return Err(invalid("当前房间没有动态直播清单"));
    }
    let mut periods = root.children().filter(|node| node.has_tag_name("Period"));
    let period = periods
        .next()
        .ok_or_else(|| invalid("DASH 清单没有 Period"))?;
    if periods.next().is_some() {
        return Err(invalid("当前不支持多 Period 直播清单"));
    }
    let base = base_url(period, &base_url(root, effective)?)?;
    let video = representation(period, &base, "video", quality)?;
    let audio = representation(period, &base, "audio", quality)?;
    Ok(Manifest {
        video,
        audio,
        period: period.attribute("id").unwrap_or_default().to_owned(),
        refreshed: Instant::now(),
    })
}

fn base_url(node: Node<'_, '_>, fallback: &Url) -> io::Result<Url> {
    match node
        .children()
        .find(|child| child.has_tag_name("BaseURL"))
        .and_then(|child| child.text())
    {
        Some(value) => media_url(fallback, value.trim()),
        None => Ok(fallback.clone()),
    }
}

fn number(node: Node<'_, '_>, name: &str, fallback: u64) -> io::Result<u64> {
    node.attribute(name).map_or(Ok(fallback), |value| {
        value.parse().map_err(|_| invalid("DASH 数值字段无效"))
    })
}

fn representation(
    period: Node<'_, '_>,
    base: &Url,
    kind: &str,
    quality: Quality,
) -> io::Result<Representation> {
    let adaptation = period
        .children()
        .find(|node| {
            node.has_tag_name("AdaptationSet")
                && (node.attribute("contentType") == Some(kind)
                    || node
                        .attribute("mimeType")
                        .is_some_and(|mime| mime.starts_with(kind)))
        })
        .ok_or_else(|| invalid("DASH 清单缺少音频或视频轨道"))?;
    let target = target_height(quality);
    let candidates: Vec<_> = adaptation
        .children()
        .filter(|node| node.has_tag_name("Representation"))
        .filter(|node| {
            node.attribute("codecs").is_some_and(|codec| {
                if kind == "video" {
                    codec.starts_with("avc1")
                } else {
                    codec == "mp4a.40.2"
                }
            })
        })
        .collect();
    let selected = candidates
        .into_iter()
        .min_by_key(|node| {
            let height = node
                .attribute("height")
                .or_else(|| adaptation.attribute("height"))
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(0);
            let bandwidth = node
                .attribute("bandwidth")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            (height > target, height.abs_diff(target), bandwidth)
        })
        .ok_or_else(|| invalid("当前只支持 H.264 和 AAC 直播轨道"))?;
    let base = base_url(selected, &base_url(adaptation, base)?)?;
    let template = selected
        .children()
        .find(|node| node.has_tag_name("SegmentTemplate"))
        .or_else(|| {
            adaptation
                .children()
                .find(|node| node.has_tag_name("SegmentTemplate"))
        })
        .ok_or_else(|| invalid("DASH 清单缺少分片模板"))?;
    let scale =
        u32::try_from(number(template, "timescale", 1)?).map_err(|_| invalid("DASH 时间基过大"))?;
    if scale == 0 {
        return Err(invalid("DASH 时间基不能为零"));
    }
    let id = selected
        .attribute("id")
        .ok_or_else(|| invalid("DASH 轨道没有编号"))?;
    let init = template
        .attribute("initialization")
        .ok_or_else(|| invalid("DASH 轨道没有初始化地址"))?;
    let pattern = template
        .attribute("media")
        .ok_or_else(|| invalid("DASH 轨道没有分片地址"))?;
    let timeline = template
        .children()
        .find(|node| node.has_tag_name("SegmentTimeline"))
        .ok_or_else(|| invalid("DASH 清单缺少分片时间轴"))?;
    let mut sequence = number(template, "startNumber", 1)?;
    let mut time = 0;
    let mut segments = BTreeMap::new();
    for segment in timeline.children().filter(|node| node.has_tag_name("S")) {
        time = number(segment, "t", time)?;
        let duration = number(segment, "d", 0)?;
        let repeat = number(segment, "r", 0)?;
        if duration == 0 || repeat > 4096 || segments.len() + repeat as usize + 1 > 4096 {
            return Err(invalid("DASH 分片时间轴无效或过长"));
        }
        // S.r 表示额外重复次数，因此该条时间轴记录共展开 r + 1 个分片。
        for _ in 0..=repeat {
            let name = pattern
                .replace("$RepresentationID$", id)
                .replace("$Number$", &sequence.to_string())
                .replace("$Time$", &time.to_string());
            if name.contains('$') {
                return Err(invalid("不支持的 DASH 分片模板"));
            }
            segments.insert(
                sequence,
                Segment {
                    url: media_url(&base, &name)?,
                    duration,
                },
            );
            sequence = sequence
                .checked_add(1)
                .ok_or_else(|| invalid("DASH 分片编号溢出"))?;
            time = time
                .checked_add(duration)
                .ok_or_else(|| invalid("DASH 时间戳溢出"))?;
        }
    }
    let init = init.replace("$RepresentationID$", id);
    if init.contains('$') {
        return Err(invalid("不支持的 DASH 初始化段模板"));
    }
    let kid = adaptation
        .children()
        .filter(|node| node.has_tag_name("ContentProtection"))
        .flat_map(|node| node.attributes())
        .find(|attribute| attribute.name() == "default_KID")
        .map(|attribute| hex_key(attribute.value()))
        .transpose()?;
    let height = u32::try_from(number(
        selected,
        "height",
        number(adaptation, "height", 0)?,
    )?)
    .map_err(|_| invalid("DASH 分辨率过大"))?;
    let bandwidth = number(selected, "bandwidth", 0)?;
    if (kind == "video" && height == 0) || bandwidth == 0 {
        return Err(invalid("DASH 轨道缺少分辨率或带宽描述"));
    }
    Ok(Representation {
        init: media_url(&base, &init)?,
        timescale: scale,
        height,
        bandwidth,
        codecs: selected.attribute("codecs").unwrap_or_default().to_owned(),
        kid,
        segments,
    })
}
