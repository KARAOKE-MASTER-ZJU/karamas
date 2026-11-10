use reqwest::ClientBuilder;
use url::{Host, Url};

/// 点歌台回环地址直连，其他地址保留客户端原有的代理配置。
pub fn bypass_loopback_proxy(builder: ClientBuilder, server_url: &str) -> ClientBuilder {
    if is_loopback_url(server_url) {
        builder.no_proxy()
    } else {
        builder
    }
}

fn is_loopback_url(server_url: &str) -> bool {
    Url::parse(server_url).is_ok_and(|url| match url.host() {
        Some(Host::Domain(host)) => host
            .strip_suffix('.')
            .unwrap_or(host)
            .eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, HttpRequest, HttpResponse, HttpServer, web};
    use std::time::Duration;

    // 回环地址属于本机；局域网地址和名称含 localhost 的远端域名不能被一并绕过。
    #[test]
    fn only_loopback_room_addresses_bypass_proxies() {
        for url in [
            "http://localhost:5173",
            "https://LOCALHOST./room",
            "http://127.0.0.1:5173",
            "http://127.42.0.9",
            "http://127.255.255.255",
            "http://[::1]:5173",
            "http://[0:0:0:0:0:0:0:1]",
        ] {
            assert!(is_loopback_url(url), "{url}");
        }
        for url in [
            "http://192.168.1.20:5173",
            "http://10.0.0.1",
            "http://128.0.0.1",
            "http://[::2]",
            "http://localhost.example.com",
            "http://example.com/localhost",
            "http://localhost@example.com",
            "invalid",
        ] {
            assert!(!is_loopback_url(url), "{url}");
        }
    }

    // 请求必须能绕过失败代理到达本机点歌台，外部点歌台继续使用已配置的代理。
    #[actix_web::test]
    async fn loopback_requests_reach_the_room_while_other_requests_keep_the_proxy() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = HttpServer::new(|| {
            App::new().default_service(web::to(|req: HttpRequest| async move {
                if req.uri().scheme().is_some() {
                    HttpResponse::BadGateway().finish()
                } else {
                    HttpResponse::Ok().finish()
                }
            }))
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .run();
        let handle = server.handle();
        actix_web::rt::spawn(server);
        let proxy = format!("http://{address}");
        for (url, expected) in [
            (format!("http://{address}/api/roomExists"), 200),
            (
                format!("http://LOCALHOST:{}/api/songListInfo", address.port()),
                200,
            ),
            ("http://room.example.invalid/api/songListInfo".into(), 502),
            ("http://192.168.1.20/api/songListInfo".into(), 502),
        ] {
            let builder = reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(&proxy).unwrap())
                .timeout(Duration::from_secs(3));
            let client = bypass_loopback_proxy(builder, &url).build().unwrap();
            let response = client.get(&url).send().await.unwrap();
            assert_eq!(response.status().as_u16(), expected, "{url}");
        }
        handle.stop(true).await;
    }
}
