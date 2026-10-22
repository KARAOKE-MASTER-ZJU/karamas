use serde::Serialize;

#[derive(Debug)]
pub(crate) struct CurrentSongSnapshot {
    pub song_id: String,
    pub song_url: String,
    pub song_title: String,
    pub room_hash: String,
}

#[derive(Debug, Serialize)]
pub struct PartError {
    pub code: &'static str,
    pub message: String,
}

impl PartError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for PartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PartError {}
