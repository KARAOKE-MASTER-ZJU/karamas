use serde::Serialize;
use std::fmt;

/// Tauri 命令统一返回的错误，保留核心库已有的业务错误码。
#[derive(Debug, Serialize)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
}

impl CommandError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CommandError {}

// 通用核心错误仅提供错误说明，在 IPC 边界统一使用 CORE_ERROR。
impl From<karamas_core::CoreError> for CommandError {
    fn from(error: karamas_core::CoreError) -> Self {
        Self::new("CORE_ERROR", error.to_string())
    }
}

// 分 P 错误已有业务码，直接保留以便调用方区分歌曲变化等情况。
impl From<karamas_core::playlist::PartError> for CommandError {
    fn from(error: karamas_core::playlist::PartError) -> Self {
        Self::new(error.code, error.message)
    }
}

/// 成功值作为 invoke 结果，错误以 { code, message } 返回给 catch。
pub type CommandResult<T> = Result<T, CommandError>;
