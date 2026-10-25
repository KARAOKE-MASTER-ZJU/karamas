//! 媒体代理的运行生命周期和 HTTP 路由。
mod lifecycle;
mod routes;

pub(crate) use lifecycle::{start, stop};
pub use routes::proxy_handler;

use crate::media::session;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct SharedState {
    pub duration_cache: Arc<Mutex<std::collections::HashMap<String, u32>>>,
    pub eplus_auth: Arc<tokio::sync::Mutex<Option<String>>>,
    pub media_sessions: Arc<session::MediaSessions>,
}
