//! Source-specific request headers for media fetched by the local DLNA proxy.
pub fn user_agent(url: &str) -> &'static str {
    // Keep the media request compatible with youtube_parser's selected client.
    // A generic Mozilla/5.0 UA can read the start but fails on later ranges.
    if referer(url) == "https://www.youtube.com/" {
        innertube_rs::constants::clients::VISIONOS_USER_AGENT
    } else {
        "Mozilla/5.0"
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn youtube_headers_apply_only_to_googlevideo_hosts() {
        for url in [
            "https://rr1.googlevideo.com/videoplayback",
            "https://googlevideo.com/videoplayback",
        ] {
            assert_eq!(
                super::user_agent(url),
                innertube_rs::constants::clients::VISIONOS_USER_AGENT
            );
            assert_eq!(super::referer(url), "https://www.youtube.com/");
        }
        for url in [
            "https://upos.bilivideo.com/video",
            "https://googlevideo.com.evil.test/video",
            "invalid",
        ] {
            assert_eq!(super::user_agent(url), "Mozilla/5.0");
            assert_eq!(super::referer(url), "https://www.bilibili.com/");
        }
    }
}

pub fn referer(url: &str) -> &'static str {
    match url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
    {
        Some(host) if host == "googlevideo.com" || host.ends_with(".googlevideo.com") => {
            "https://www.youtube.com/"
        }
        _ => "https://www.bilibili.com/",
    }
}
