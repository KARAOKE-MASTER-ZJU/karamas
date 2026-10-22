//! 房间歌单、同步驱动和切歌进度保护。
mod auto_next;
mod commands;
mod manager;
pub(crate) mod parts;
pub(crate) mod progress_gate;
mod sync;

pub(crate) use commands::SwitchSongError;
pub use manager::PlaylistManager;
pub use parts::PartError;
