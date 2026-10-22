//! HTTP representation of an immutable, seekable MP4. Range offsets always
//! refer to this output, never directly to one of its audio/video source files.
use crate::{
    seekable_mp4::{ReadPart, VirtualMp4},
    song_cache::{BLOCK_BYTES, SongCache},
};
use actix_web::{HttpRequest, HttpResponse, http::Method};
use futures_util::StreamExt;
use std::{ops::Range, sync::Arc, time::Instant};

#[derive(Debug, PartialEq, Eq)]
enum RequestedRange {
    Full,
    Partial(Range<u64>),
    Unsatisfiable,
}

fn resolve_range(value: &str, length: u64) -> RequestedRange {
    let Some(value) = value.strip_prefix("bytes=") else {
        return RequestedRange::Full;
    };
    // A server may ignore unsupported multipart ranges; never return a wrong
    // single-range 206 for a multi-range request.
    if value.contains(',') {
        return RequestedRange::Full;
    }
    let Some((start, end)) = value.split_once('-') else {
        return RequestedRange::Full;
    };
    let number = |s: &str| {
        if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
            s.parse::<u64>().ok()
        } else {
            None
        }
    };
    if start.is_empty() {
        let Some(suffix) = number(end) else {
            return RequestedRange::Full;
        };
        if suffix == 0 || length == 0 {
            return RequestedRange::Unsatisfiable;
        }
        return RequestedRange::Partial(length.saturating_sub(suffix)..length);
    }
    let Some(start) = number(start) else {
        return RequestedRange::Full;
    };
    let end = if end.is_empty() {
        length.saturating_sub(1)
    } else {
        let Some(end) = number(end) else {
            return RequestedRange::Full;
        };
        if end < start {
            return RequestedRange::Full;
        }
        end
    };
    if start >= length {
        return RequestedRange::Unsatisfiable;
    }
    RequestedRange::Partial(start..end.min(length - 1) + 1)
}

/// `generation` identifies one immutable media representation, including quality.
/// The session owner must reject URLs for obsolete generations before calling us.
pub fn serve(
    req: &HttpRequest,
    mp4: Arc<VirtualMp4>,
    cache: Arc<SongCache>,
    generation: u64,
) -> HttpResponse {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return HttpResponse::MethodNotAllowed()
            .insert_header(("Allow", "GET, HEAD"))
            .finish();
    }
    let etag = format!("\"ktv-{generation}\"");
    let h = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok());
    if h("If-None-Match").is_some_and(|v| {
        v.split(',')
            .map(str::trim)
            .any(|tag| tag == etag || tag == "*" || tag.strip_prefix("W/") == Some(etag.as_str()))
    }) {
        return HttpResponse::NotModified()
            .insert_header(("ETag", etag))
            .finish();
    }
    let requested = if req.method() == Method::GET && h("If-Range").is_none_or(|v| v == etag) {
        h("Range").map_or(RequestedRange::Full, |v| resolve_range(v, mp4.len))
    } else {
        RequestedRange::Full
    };
    let mut response;
    let range = match requested {
        RequestedRange::Unsatisfiable => {
            return HttpResponse::RangeNotSatisfiable()
                .insert_header(("Content-Range", format!("bytes */{}", mp4.len)))
                .insert_header(("Accept-Ranges", "bytes"))
                .insert_header(("ETag", etag))
                .finish();
        }
        RequestedRange::Full => {
            response = HttpResponse::Ok();
            0..mp4.len
        }
        RequestedRange::Partial(range) => {
            response = HttpResponse::PartialContent();
            response.insert_header((
                "Content-Range",
                format!("bytes {}-{}/{}", range.start, range.end - 1, mp4.len),
            ));
            range
        }
    };
    response
        .insert_header(("Content-Type", "video/mp4"))
        .insert_header(("Content-Length", (range.end - range.start).to_string()))
        .insert_header(("Accept-Ranges", "bytes"))
        .insert_header(("ETag", etag))
        .insert_header(("transferMode.dlna.org", "Streaming"))
        .insert_header(("contentFeatures.dlna.org", "DLNA.ORG_OP=01;DLNA.ORG_CI=0"));
    if req.method() == Method::HEAD {
        return response.finish();
    }
    let parts = match mp4.read_plan(range.clone()) {
        Ok(parts) => parts,
        Err(e) => {
            log::error!(target: "DLNA1080", "MP4 字节映射失败: {}", e);
            return HttpResponse::InternalServerError().finish();
        }
    };
    log::debug!(target: "DLNA1080", "MP4 范围请求: generation={}, start={}, end={}", generation, range.start, range.end);
    // Poll ahead across interleaved video/audio samples, while preserving exact
    // output order. The source cache deduplicates identical block requests and
    // bounds network concurrency. Dropping this stream cancels its read-ahead.
    let parts = parts.into_iter().flat_map(|part| {
        let mut part = Some(part);
        std::iter::from_fn(move || match part.take()? {
            ReadPart::Header(r) => Some(ReadPart::Header(r)),
            ReadPart::Source { source, range } => {
                let end = range.end.min(range.start.saturating_add(BLOCK_BYTES));
                if end < range.end {
                    part = Some(ReadPart::Source {
                        source,
                        range: end..range.end,
                    });
                }
                Some(ReadPart::Source {
                    source,
                    range: range.start..end,
                })
            }
        })
    });
    let started = Instant::now();
    let (stream_id, mut epoch) = cache.stream_epoch();
    let mut emitted = 0u64;
    let mut reported = false;
    let range_start = range.start;
    let stream = futures_util::stream::iter(parts)
        .map(move |part| {
            let cache = cache.clone();
            let mp4 = mp4.clone();
            async move {
                match part {
                    ReadPart::Header(range) => Ok(mp4.prefix.slice(range)),
                    ReadPart::Source { source, range } => cache.read(source, range).await,
                }
            }
        })
        .buffered(32)
        .inspect(move |result| {
            if let Ok(bytes) = result {
                if emitted == 0 {
                    log::debug!(target: "DLNA1080", "MP4 首字节: generation={}, start={}, ready_ms={}", generation, range_start, started.elapsed().as_millis());
                }
                emitted += bytes.len() as u64;
                if !reported && emitted >= 128 * 1024 {
                    reported = true;
                    log::debug!(target: "DLNA1080", "MP4 首128KiB: generation={}, start={}, ready_ms={}", generation, range_start, started.elapsed().as_millis());
                }
            }
        })
        .take_until(async move {
            loop {
                if *epoch.borrow_and_update() >= stream_id { break; }
                if epoch.changed().await.is_err() { break; }
            }
        });
    response.streaming(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_inclusive_open_suffix_and_clipped_ranges() {
        assert_eq!(
            resolve_range("bytes=0-0", 10),
            RequestedRange::Partial(0..1)
        );
        assert_eq!(
            resolve_range("bytes=3-", 10),
            RequestedRange::Partial(3..10)
        );
        assert_eq!(
            resolve_range("bytes=-3", 10),
            RequestedRange::Partial(7..10)
        );
        assert_eq!(
            resolve_range("bytes=-30", 10),
            RequestedRange::Partial(0..10)
        );
        assert_eq!(
            resolve_range("bytes=7-100", 10),
            RequestedRange::Partial(7..10)
        );
        assert_eq!(
            resolve_range("bytes=7-18446744073709551615", 10),
            RequestedRange::Partial(7..10)
        );
    }
    #[test]
    fn distinguishes_invalid_unsupported_and_unsatisfiable_ranges() {
        for input in ["bytes=10-", "bytes=-0", "bytes=20-30"] {
            assert_eq!(resolve_range(input, 10), RequestedRange::Unsatisfiable);
        }
        for input in [
            "bytes=5-2",
            "bytes=-",
            "bytes=+2-4",
            "bytes=0-2,5-8",
            "items=0-1",
            "bytes=18446744073709551616-",
        ] {
            assert_eq!(resolve_range(input, 10), RequestedRange::Full);
        }
        assert_eq!(resolve_range("bytes=0-", 0), RequestedRange::Unsatisfiable);
    }
}
