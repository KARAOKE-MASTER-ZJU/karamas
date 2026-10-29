//! 核心库与宿主共用的 HTTP 客户端配置。

use std::sync::{Arc, OnceLock};

/// 在宿主启动时调用，供 Tauri 移动端开发代理等自行创建客户端的依赖使用。
pub fn init_crypto_provider() {
    // 已有 provider 时保留宿主的选择。
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// 创建使用 ring 和 WebPKI 根证书的 HTTP 客户端构建器。
pub fn client_builder() -> reqwest::ClientBuilder {
    static TLS: OnceLock<rustls::ClientConfig> = OnceLock::new();
    let tls = TLS.get_or_init(|| {
        let roots =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("ring 支持默认 TLS 协议版本")
        .with_root_certificates(roots)
        .with_no_client_auth();
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        config
    });
    reqwest::Client::builder().tls_backend_preconfigured(tls.clone())
}
