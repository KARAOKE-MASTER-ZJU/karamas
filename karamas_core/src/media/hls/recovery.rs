use super::{Track, invalid, load_index};
use bytes::Bytes;
use std::{future::Future, io, ops::Range, time::Duration};
use tokio::sync::Mutex;

impl Track {
    pub(super) async fn recover<T, F, Fut>(&mut self, budget: Duration, load: F) -> io::Result<T>
    where
        F: FnMut(String, bool) -> Fut,
        Fut: Future<Output = io::Result<T>>,
    {
        let refresh = self.refresh.clone();
        self.recover_with(budget, load, async move {
            match refresh {
                Some(refresh) => refresh.resolve().await.map(Some).map_err(io::Error::other),
                None => Ok(None),
            }
        })
        .await
    }

    async fn recover_with<T, F, Fut, R>(
        &mut self,
        budget: Duration,
        mut load: F,
        refresh: R,
    ) -> io::Result<T>
    where
        F: FnMut(String, bool) -> Fut,
        Fut: Future<Output = io::Result<T>>,
        R: Future<Output = io::Result<Option<Track>>>,
    {
        let mut candidate = self.clone();
        let mut refresh = Some(refresh);
        let recover = async {
            let mut last = io::Error::other("HLS 没有可用的媒体地址");
            for refreshed in [false, true] {
                match candidate
                    .try_urls(
                        (budget.saturating_sub(crate::config::API_TIMEOUT)) / 2,
                        refreshed,
                        &mut load,
                    )
                    .await
                {
                    Ok(value) => {
                        *self = candidate;
                        return Ok(value);
                    }
                    Err(error) => last = error,
                }
                if refreshed {
                    break;
                }
                let Some(refresh) = refresh.take() else {
                    break;
                };
                let Some(fresh) = refresh.await? else {
                    break;
                };
                log::info!(target: "HLS", "HLS 已重新取源，尝试恢复下载");
                // 同一会话的字节偏移固定，重新取源不能切换编码或轨道布局。
                if fresh.codecs != candidate.codecs
                    || fresh.bandwidth != candidate.bandwidth
                    || fresh.initialization != candidate.initialization
                    || fresh.index != candidate.index
                {
                    return Err(invalid("重新取源后的 HLS 轨道已变化，请重新播放"));
                }
                candidate = fresh;
            }
            Err(io::Error::new(
                last.kind(),
                format!("HLS 媒体恢复失败：{last:?}"),
            ))
        };
        tokio::time::timeout(budget, recover)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "HLS 媒体恢复超时，请重新播放"))?
    }

    async fn try_urls<T, F, Fut>(
        &mut self,
        budget: Duration,
        refreshed: bool,
        load: &mut F,
    ) -> io::Result<T>
    where
        F: FnMut(String, bool) -> Fut,
        Fut: Future<Output = io::Result<T>>,
    {
        let attempt_urls = async {
            let mut last = io::Error::other("HLS 没有可用的媒体地址");
            for (index, url) in self.urls.clone().into_iter().enumerate() {
                for attempt in 0..2 {
                    match load(url.clone(), refreshed).await {
                        Ok(value) => {
                            self.urls.rotate_left(index);
                            return Ok(value);
                        }
                        Err(error) => {
                            let retry = matches!(
                                error.kind(),
                                io::ErrorKind::TimedOut
                                    | io::ErrorKind::ConnectionReset
                                    | io::ErrorKind::ConnectionAborted
                                    | io::ErrorKind::ConnectionRefused
                                    | io::ErrorKind::UnexpectedEof
                                    | io::ErrorKind::Other
                            );
                            let endpoint = url::Url::parse(&url)
                                .ok()
                                .map(|url| url.origin().ascii_serialization())
                                .unwrap_or_default();
                            log::warn!(target: "HLS", "HLS 请求失败：endpoint={}, attempt={}, refreshed={}, error={:?}", endpoint, attempt + 1, refreshed, error);
                            last = error;
                            if !retry || attempt == 1 {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    }
                }
            }
            Err(last)
        };
        // 为重新取源保留时间，避免失效 CDN 耗尽整个恢复窗口。
        tokio::time::timeout(budget, attempt_urls)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "HLS CDN 请求超时"))?
    }
}

pub(crate) struct Source {
    pub(super) url: String,
    track: Mutex<Track>,
    pub(crate) total: u64,
    index: Vec<u8>,
}

impl Source {
    pub(crate) fn new(track: Track, total: u64, index: Vec<u8>) -> Self {
        Self {
            url: track.urls[0].clone(),
            track: Mutex::new(track),
            total,
            index,
        }
    }

    pub(crate) async fn read(
        &self,
        client: &reqwest::Client,
        range: Range<u64>,
    ) -> io::Result<Bytes> {
        // 每次只读一个有界块；同轨道共用成功的 CDN 和刷新结果。
        let mut track = self.track.lock().await;
        let original = track.clone();
        track
            .recover(Duration::from_secs(20), |url, refreshed| {
                let range = range.clone();
                let original = &original;
                async move {
                    // 在播放器的加载时限内留出重试、换线和重新取源的时间。
                    tokio::time::timeout(Duration::from_secs(3), async {
                        if refreshed {
                            let (index, total) = load_index(client, original, &url).await?;
                            if total != self.total || index != self.index {
                                return Err(invalid("重新取源后的 HLS 索引已变化，请重新播放"));
                            }
                        }
                        crate::media::upstream::fetch_range(client, &url, range, self.total).await
                    })
                    .await
                    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "读取 HLS 媒体超时"))?
                }
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 地址过期可以换源，已发布的轨道布局不能随恢复改变，失败也不能污染后续请求。
    #[tokio::test]
    async fn expired_urls_refresh_without_replacing_the_published_representation() {
        let original = Track::new(
            "https://expired.example/track".into(),
            "avc1.640033",
            1000,
            "0-15",
            "16-59",
        )
        .unwrap();
        let mut fresh = original.clone();
        fresh.urls = vec!["https://fresh.example/track".into()];
        let load = |url: String, _| async move {
            if url.contains("expired.example") {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "HTTP 403"))
            } else {
                Ok("完整媒体")
            }
        };
        let mut track = original.clone();
        assert_eq!(
            track
                .recover_with(Duration::from_secs(10), load, async {
                    Ok(Some(fresh.clone()))
                })
                .await
                .unwrap(),
            "完整媒体"
        );
        let mut changed = fresh;
        changed.codecs = "hev1.1.6.L120.90".into();
        let mut track = original.clone();
        let error = track
            .recover_with(Duration::from_secs(10), load, async { Ok(Some(changed)) })
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(track.urls, original.urls);
    }
}
