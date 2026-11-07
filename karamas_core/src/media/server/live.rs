//! 直播资源沿用当前媒体会话的 token、设备和代次校验。
use super::hls::{cors, serve_track_with_referer};
use crate::media::{
    live::{LiveMedia, Resource},
    session::MediaSession,
};
use actix_files::NamedFile;
use actix_web::{
    body::{BodySize, BoxBody, MessageBody},
    HttpRequest, HttpResponse,
};
use bytes::Bytes;
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

pub(super) async fn serve(
    req: &HttpRequest,
    client: &reqwest::Client,
    session: Arc<MediaSession>,
    media: &LiveMedia,
    name: &str,
) -> Result<HttpResponse, actix_web::Error> {
    let resource = tokio::select! {
        biased;
        _ = session.cancelled() => return Ok(cors(&mut HttpResponse::Gone()).finish()),
        resource = media.resource(client, name) => resource,
    };
    let resource = match resource {
        Ok(resource) => resource,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(cors(&mut HttpResponse::Gone()).finish());
        }
        Err(error) => return Err(actix_web::error::ErrorBadGateway(error)),
    };
    match resource {
        Resource::Remote(url) => {
            serve_track_with_referer(req, client, session, url.as_str(), media.referer()).await
        }
        Resource::Playlist(text) => {
            let mut response = HttpResponse::Ok();
            cors(&mut response)
                .insert_header(("Content-Type", "application/vnd.apple.mpegurl"))
                .insert_header(("Cache-Control", "no-store"))
                .insert_header(("Content-Length", text.len().to_string()));
            // actix 在 HEAD 请求中省略响应体，同时从正文保留正确的清单长度。
            Ok(response.body(text))
        }
        Resource::File(file) => {
            let response_file = match NamedFile::from_file(file.reopen()?, file.path()) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(cors(&mut HttpResponse::Gone()).finish());
                }
                Err(error) => return Err(actix_web::error::ErrorInternalServerError(error)),
            };
            // 分片的 HEAD、Range 和条件请求交给 actix-files。
            let mut response = response_file
                .disable_content_disposition()
                .set_content_type(
                    if name.starts_with("audio-") {
                        "audio/mp4"
                    } else {
                        "video/mp4"
                    }
                    .parse()
                    .unwrap(),
                )
                .into_response(req);
            response.headers_mut().insert(
                actix_web::http::header::CACHE_CONTROL,
                "no-store".parse().unwrap(),
            );
            for (name, value) in [
                ("access-control-allow-origin", "*"),
                (
                    "access-control-expose-headers",
                    "Content-Length, Content-Range, Accept-Ranges",
                ),
            ] {
                response
                    .headers_mut()
                    .insert(name.parse().unwrap(), value.parse().unwrap());
            }
            Ok(response
                .map_body(move |_, body| FileBody {
                    body,
                    _file: file,
                    session,
                })
                .map_into_boxed_body())
        }
    }
}

// 缓存淘汰后，响应仍持有临时文件，读完再清理；Windows 也能按此顺序关闭和删除。
struct FileBody {
    body: BoxBody,
    _file: Arc<tempfile::NamedTempFile>,
    session: Arc<MediaSession>,
}

impl MessageBody for FileBody {
    type Error = <BoxBody as MessageBody>::Error;
    fn size(&self) -> BodySize {
        self.body.size()
    }
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        if self.session.is_stopped() {
            return Poll::Ready(None);
        }
        Pin::new(&mut self.body).poll_next(cx)
    }
}
