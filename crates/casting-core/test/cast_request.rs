use super::retry_cast_request;
use std::time::Duration;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Copy)]
enum Reply {
    Bytes(&'static [u8]),
    Timeout,
    SlowBody,
}

#[tokio::test]
async fn control_requests_retry_only_timeouts_before_response_headers() {
    const OK: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
    const INVALID: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\n{";
    const TRUNCATED: &[u8] =
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{";
    const REJECTED: &[u8] = b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    for (replies, attempts, succeeds, timed_out) in [
        (vec![Reply::Bytes(OK)], 1, true, false),
        (
            vec![Reply::Timeout, Reply::Timeout, Reply::Bytes(OK)],
            3,
            true,
            false,
        ),
        (vec![Reply::Timeout; 4], 3, false, true),
        (vec![Reply::Bytes(b""), Reply::Bytes(OK)], 1, false, false),
        (vec![Reply::Bytes(INVALID)], 1, false, false),
        (vec![Reply::Bytes(REJECTED)], 1, false, false),
        (
            vec![Reply::Bytes(TRUNCATED), Reply::Bytes(OK)],
            1,
            false,
            false,
        ),
        (vec![Reply::SlowBody, Reply::Bytes(OK)], 1, false, true),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let server = tokio::spawn(async move {
            let mut replies = VecDeque::from(replies);
            let mut connections = tokio::task::JoinSet::new();
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                observed.fetch_add(1, Ordering::SeqCst);
                let reply = replies.pop_front().unwrap();
                connections.spawn(async move {
                    let mut request = [0; 4096];
                    stream.read(&mut request).await.unwrap();
                    match reply {
                        Reply::Bytes(bytes) => stream.write_all(bytes).await.unwrap(),
                        Reply::Timeout => tokio::time::sleep(Duration::from_millis(500)).await,
                        Reply::SlowBody => {
                            stream.write_all(TRUNCATED).await.unwrap();
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                    }
                });
            }
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(200))
            .build()
            .unwrap();
        let result = async {
            retry_cast_request("test request", || client.post(&url).send())
                .await?
                .error_for_status()?
                .json::<serde_json::Value>()
                .await
        }
        .await;
        server.abort();
        assert_eq!(calls.load(Ordering::SeqCst), attempts);
        assert_eq!(result.is_ok(), succeeds, "{result:?}");
        assert_eq!(
            result.as_ref().err().is_some_and(|e| e.is_timeout()),
            timed_out,
            "{result:?}"
        );
    }
}

#[tokio::test]
async fn connection_refused_is_not_retried() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let calls = AtomicUsize::new(0);
    let error = retry_cast_request("test request", || {
        calls.fetch_add(1, Ordering::SeqCst);
        client.post(&url).send()
    })
    .await
    .unwrap_err();
    assert!(error.is_connect());
    assert!(!error.is_timeout());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
