use crate::{cast, error::CoreError, playlist::PlaylistManager};
use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;

pub static ENGINE_STATE: RwLock<Option<Arc<EngineContext>>> = RwLock::new(None);

pub struct EngineContext {
    pub caster: Arc<dyn cast::Caster>,
    pub playlist_manager: PlaylistManager,
    pub duration_cache: Arc<Mutex<std::collections::HashMap<String, u32>>>,
    pub local_ip: std::net::IpAddr,
    pub server_port: u16,
    pub is_playing: AtomicBool,
    pub(super) playback_lock: Arc<Mutex<()>>,
    /// 仅 Bilibili 投屏使用：本地跟踪的弹幕/清晰度状态（设备侧没有读回接口）。
    // pub danmaku_on: AtomicBool,
    // pub quality_qn: AtomicU32,
    /// 调用方需持有运行时，直到引擎停止。
    pub rt: tokio::runtime::Handle,
    pub(super) tasks: std::sync::Mutex<Vec<tokio::task::AbortHandle>>,
    pub(super) stopped: AtomicBool,
}

impl EngineContext {
    pub fn is_playing(&self) -> bool {
        self.caster
            .playback_snapshot()
            .map(|state| state.is_playing)
            .unwrap_or_else(|| self.is_playing.load(Ordering::SeqCst))
    }

    pub(super) fn spawn_task<F>(&self, task: F) -> Result<tokio::task::JoinHandle<F::Output>, CoreError>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let mut tasks = self.tasks.lock().unwrap_or_else(|error| error.into_inner());
        if self.stopped.load(Ordering::SeqCst) {
            return Err("Engine stopped".into());
        }
        tasks.retain(|task| !task.is_finished());
        let task = self.rt.spawn(task);
        tasks.push(task.abort_handle());
        Ok(task)
    }

    pub(super) fn stop_tasks(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        self.caster.stop_status_subscription();
        let tasks =
            std::mem::take(&mut *self.tasks.lock().unwrap_or_else(|error| error.into_inner()));
        for task in tasks {
            task.abort();
        }
        let gate = self.playlist_manager.progress_gate.clone();
        if let Ok(mut gate) = gate.try_lock() {
            gate.auto_next.cancel_pending_request();
        } else {
            self.rt.spawn(async move {
                gate.lock().await.auto_next.cancel_pending_request();
            });
        }
    }
}

impl Drop for EngineContext {
    fn drop(&mut self) {
        self.stop_tasks();
    }
}
