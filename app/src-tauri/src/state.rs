use crate::error::{CommandError, CommandResult};
use std::{path::PathBuf, sync::Arc};
use tokio::{
    sync::{Mutex, OnceCell},
    task::AbortHandle,
};

/// 只管理 Tauri 接入所需的资源，播放状态仍由核心库持有。
pub struct BridgeState {
    /// 串行执行启动和断开，避免同时修改核心库的全局会话。
    pub lifecycle: Mutex<()>,
    /// 当前扫码任务的取消句柄，不保存登录结果。
    pub login: Mutex<Option<AbortHandle>>,
    /// 串行执行会话文件的读取、保存和删除。
    pub session_io: Mutex<()>,
    /// 初始化成功后复用；失败时允许后续调用重试。
    storage_ready: OnceCell<()>,
    data_dir: PathBuf,
    cache_dir: PathBuf,
}

impl BridgeState {
    /// 记录应用私有目录，目录创建延迟到首次使用时。
    pub fn new(data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            lifecycle: Mutex::new(()),
            login: Mutex::new(None),
            session_io: Mutex::new(()),
            storage_ready: OnceCell::new(),
            data_dir,
            cache_dir,
        }
    }

    /// 首次使用时在阻塞线程池初始化应用数据和媒体缓存目录。
    pub async fn initialize_storage(&self) -> CommandResult<()> {
        self.storage_ready
            .get_or_try_init(|| async {
                let data_dir = self.data_dir.clone();
                let cache_dir = self.cache_dir.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    std::fs::create_dir_all(&data_dir)
                        .map_err(|e| CommandError::new("STORAGE_ERROR", e.to_string()))?;
                    let path = data_dir.to_str().ok_or_else(|| {
                        CommandError::new("STORAGE_ERROR", "应用数据路径不是有效的 UTF-8")
                    })?;
                    karamas_core::cast::bilibili::init_session_dir(path);
                    let cache_path = cache_dir.to_str().ok_or_else(|| {
                        CommandError::new("STORAGE_ERROR", "媒体缓存路径不是有效的 UTF-8")
                    })?;
                    karamas_core::media::session::init_cache_dir(cache_path)
                        .map_err(|e| CommandError::new("STORAGE_ERROR", e.to_string()))
                })
                .await
                .map_err(|e| CommandError::new("TASK_FAILED", e.to_string()))?
            })
            .await?;
        Ok(())
    }
}

/// 取出当前引擎后释放全局状态锁，再执行异步操作。
pub fn current_engine() -> CommandResult<Arc<karamas_core::EngineContext>> {
    karamas_core::ENGINE_STATE
        .read()
        .map_err(|_| CommandError::new("STATE_ERROR", "无法读取引擎状态"))?
        .as_ref()
        .cloned()
        .ok_or_else(|| CommandError::new("NOT_CONNECTED", "尚未连接投屏会话"))
}
