/// Retry control requests only on timeout, before receiving response headers.
/// Media preparation and response body reads stay outside this loop.
/// 
/// 重试控制请求仅在超时的情况下进行重试，直到接收到响应头为止
/// 媒体准备和响应体读取不在此循环内
pub(crate) async fn retry_cast_request<T, F, Fut>(context: &str, mut send: F) -> Result<T, reqwest::Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, reqwest::Error>>,
{
    for attempt in 0..=2 {
        match send().await {
            Err(error) if attempt < 2 && error.is_timeout() => {
                log::warn!(
                    "{}：请求超时，1 秒后重发（{}/2），错误={}",
                    context,
                    attempt + 1,
                    error
                );
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            result => return result,
        }
    }
    unreachable!("the third attempt always returns")
}

#[cfg(test)]
mod tests;
