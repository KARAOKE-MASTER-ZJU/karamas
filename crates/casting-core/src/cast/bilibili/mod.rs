//! B 站扫码认证、会话存储、设备枚举和投屏控制。
mod auth;
mod caster;
mod client;
mod devices;
mod session;

pub use auth::login_qr;
pub use caster::BilibiliCaster;
pub use devices::{BilibiliDevice, list_devices};
pub use session::{
    BilibiliSession, TOKEN_FILE, clear_session, init_session_dir, is_session_expired, load_session,
    save_session,
};
