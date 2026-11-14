use super::*;
use crate::media::mp4::test_support::*;

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
#[actix_web::test]
async fn http_head_partial_header_and_unsatisfiable_range() {
    use crate::{
        media::cache::{BLOCK_BYTES, RemoteSource, SongCache},
        media::mp4::http::serve,
    };
    use actix_web::{
        body::to_bytes,
        http::{Method, StatusCode},
        test::TestRequest,
    };
    use std::sync::Arc;
    let mut b = builder(false);
    b.add_fragment(0, 100, &fragment(0, ints(&[1, 3, 100])))
        .unwrap();
    let mp4 = Arc::new(b.finish().unwrap());
    // Requests in this test must never need a network connection.
    let cache = Arc::new(
        SongCache::new(
            &std::env::temp_dir(),
            reqwest::Client::new(),
            vec![RemoteSource {
                url: "http://127.0.0.1:1/unreachable".into(),
                len: 209,
            }],
            BLOCK_BYTES,
        )
        .unwrap(),
    );
    let req = TestRequest::default()
        .method(Method::HEAD)
        .insert_header(("Range", "bytes=0-9"))
        .to_http_request();
    let response = serve(&req, mp4.clone(), cache.clone(), 42);
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("Content-Length")
            .unwrap()
            .to_str()
            .unwrap(),
        mp4.len.to_string()
    );
    assert!(to_bytes(response.into_body()).await.unwrap().is_empty());
    let req = TestRequest::default()
        .insert_header(("Range", "bytes=3-12"))
        .to_http_request();
    let response = serve(&req, mp4.clone(), cache.clone(), 42);
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        response
            .headers()
            .get("Content-Range")
            .unwrap()
            .to_str()
            .unwrap(),
        format!("bytes 3-12/{}", mp4.len)
    );
    assert_eq!(
        to_bytes(response.into_body()).await.unwrap(),
        mp4.prefix.slice(3..13)
    );
    let req = TestRequest::default()
        .insert_header(("Range", format!("bytes={}-", mp4.len)))
        .to_http_request();
    assert_eq!(
        serve(&req, mp4.clone(), cache.clone(), 42).status(),
        StatusCode::RANGE_NOT_SATISFIABLE
    );
    let req = TestRequest::default()
        .insert_header(("Range", "bytes=3-12"))
        .insert_header(("If-Range", "\"old-file\""))
        .to_http_request();
    assert_eq!(
        serve(&req, mp4.clone(), cache.clone(), 42).status(),
        StatusCode::OK
    );
    let req = TestRequest::default()
        .insert_header(("If-None-Match", "W/\"ktv-42\""))
        .to_http_request();
    assert_eq!(
        serve(&req, mp4, cache.clone(), 42).status(),
        StatusCode::NOT_MODIFIED
    );
    assert_eq!(cache.stats().0, 0);
}

#[actix_web::test]
async fn seek_retires_a_blocked_response_and_new_range_retries_the_block() {
    use crate::{
        media::cache::{BLOCK_BYTES, RemoteSource, SongCache},
        media::mp4::http::serve,
    };
    use actix_web::{body::to_bytes, test::TestRequest};
    use std::{sync::Arc, time::Duration};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/fixture", listener.local_addr().unwrap());
    let (accepted, waiting) = tokio::sync::oneshot::channel();
    let origin = tokio::spawn(async move {
        let mut accepted = Some(accepted);
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let mut bytes = [0; 512];
                let n = socket.read(&mut bytes).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&bytes[..n]);
            }
            if attempt == 0 {
                // Hold the response until cancellation closes this request.
                accepted.take().unwrap().send(()).unwrap();
                let mut byte = [0];
                let closed = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
                    .await
                    .unwrap();
                assert!(matches!(closed, Ok(0) | Err(_)));
                continue;
            }
            let mut response = b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-208/209\r\nContent-Length: 209\r\nConnection: close\r\n\r\n".to_vec();
            response.extend_from_slice(&[0; 200]);
            response.extend_from_slice(b"abcdefghi");
            socket.write_all(&response).await.unwrap();
        }
    });
    let cache = Arc::new(
        SongCache::new(
            &std::env::temp_dir(),
            reqwest::Client::new(),
            vec![RemoteSource { url, len: 209 }],
            BLOCK_BYTES,
        )
        .unwrap(),
    );
    let mut b = builder(false);
    b.add_fragment(0, 100, &fragment(0, ints(&[1, 3, 100])))
        .unwrap();
    let mp4 = Arc::new(b.finish().unwrap());
    let request = TestRequest::default()
        .insert_header(("Range", format!("bytes={}-", mp4.prefix.len())))
        .to_http_request();
    let old = serve(&request, mp4.clone(), cache.clone(), 99);
    let old_read = actix_web::rt::spawn(async move { to_bytes(old.into_body()).await.unwrap() });
    tokio::time::timeout(Duration::from_secs(2), waiting)
        .await
        .unwrap()
        .unwrap();
    let cutoff = cache.stream_cutoff();
    let new = serve(&request, mp4, cache.clone(), 99);
    cache.retire_streams(cutoff);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), old_read)
            .await
            .unwrap()
            .unwrap()
            .is_empty()
    );
    let bytes = tokio::time::timeout(Duration::from_secs(2), to_bytes(new.into_body()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes[..], b"abcdefghi");
    origin.await.unwrap();
}

#[actix_web::test]
async fn http_full_and_cross_boundary_ranges_stream_identical_media() {
    use crate::{
        media::cache::{BLOCK_BYTES, RemoteSource, SongCache},
        media::mp4::http::serve,
    };
    use actix_web::{body::to_bytes, http::StatusCode, test::TestRequest};
    use std::sync::Arc;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/fixture", listener.local_addr().unwrap());
    let origin = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut b = [0; 512];
            let n = socket.read(&mut b).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&b[..n]);
            if request.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let mut response = b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-208/209\r\nContent-Length: 209\r\nConnection: close\r\n\r\n".to_vec();
        response.extend_from_slice(&[0; 200]);
        response.extend_from_slice(b"abcdefghi");
        socket.write_all(&response).await.unwrap();
    });
    let cache = Arc::new(
        SongCache::new(
            &std::env::temp_dir(),
            reqwest::Client::new(),
            vec![RemoteSource { url, len: 209 }],
            BLOCK_BYTES,
        )
        .unwrap(),
    );
    let mut b = builder(false);
    b.add_fragment(0, 100, &fragment(0, ints(&[1, 3, 100])))
        .unwrap();
    let mp4 = Arc::new(b.finish().unwrap());
    let full = serve(
        &TestRequest::default().to_http_request(),
        mp4.clone(),
        cache.clone(),
        43,
    );
    let data = to_bytes(full.into_body()).await.unwrap();
    assert_eq!(&data[..mp4.prefix.len()], &mp4.prefix[..]);
    assert_eq!(&data[mp4.prefix.len()..], b"abcdefghi");
    origin.await.unwrap();
    // 上游已退出，后续范围读取仍应从缓存返回相同字节。
    let start = mp4.prefix.len() - 3;
    let req = TestRequest::default()
        .insert_header(("Range", format!("bytes={start}-{}", start + 7)))
        .to_http_request();
    let response = serve(&req, mp4.clone(), cache.clone(), 43);
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        to_bytes(response.into_body()).await.unwrap(),
        data.slice(start..start + 8)
    );
}
