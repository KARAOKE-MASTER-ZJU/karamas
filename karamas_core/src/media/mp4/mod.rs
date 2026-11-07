//! MP4 索引、封装和 HTTP Range 表示。
pub mod dash;
pub mod duration;
pub mod fragmented;
pub mod http;
pub mod indexed;

pub(crate) mod boxes;
pub(crate) mod cenc;
