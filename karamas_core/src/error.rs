/// 公共接口错误，保留原始错误并支持跨线程传递。
pub type CoreError = Box<dyn std::error::Error + Send + Sync + 'static>;
