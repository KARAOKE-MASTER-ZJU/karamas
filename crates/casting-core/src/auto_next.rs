use crate::playlist_manager::{PlaylistManager, SwitchSongError};
use std::time::Duration;
use tokio::task::JoinHandle;

const END_MARGIN_SECS: i32 = 2;

/// Device play success rearms the trigger; playlist/hash updates do not.
#[derive(Default)]
pub(crate) struct AutoNextSong {
    fired: bool,
    retry_task: Option<JoinHandle<()>>,
}

impl AutoNextSong {
    pub(crate) fn reset_for_playback(&mut self) {
        self.fired = false;
        self.cancel_pending_request();
    }

    pub(crate) fn cancel_pending_request(&mut self) {
        if let Some(task) = self.retry_task.take() {
            task.abort();
        }
    }

    pub(crate) fn observe(&mut self, current: i32, total: i32) -> bool {
        if current > 5
            && total > 0
            && total.saturating_sub(current) <= END_MARGIN_SECS
            && !self.fired
        {
            self.fired = true;
            return true;
        }
        false
    }

    pub(crate) fn start_request(
        &mut self,
        manager: PlaylistManager,
        hash: String,
        runtime: &tokio::runtime::Handle,
    ) {
        // Keep only the HTTP request, not the manager owning this trigger.
        let request = manager.switch_request(true, &hash);
        self.retry_task = Some(runtime.spawn(retry_next_song(request)));
    }
}

impl Drop for AutoNextSong {
    fn drop(&mut self) {
        if let Some(task) = self.retry_task.take() {
            task.abort();
        }
    }
}

async fn retry_next_song(request: reqwest::RequestBuilder) {
    loop {
        let retry = request
            .try_clone()
            .expect("JSON switch request is cloneable");
        match PlaylistManager::send_switch_request(retry).await {
            Ok(()) => return,
            Err(SwitchSongError::Rejected(error)) => {
                // A lost success response may be followed by REJECT on retry.
                // Leave playlist synchronization to the existing WS/polling loop.
                log::info!("自动切歌请求已结束，等待歌单同步: {}", error);
                return;
            }
            Err(SwitchSongError::Retryable(error)) => {
                log::warn!("自动切歌暂时失败，1 秒后使用原 hash 重试: {}", error);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

#[cfg(test)]
#[path = "../test/auto_next.rs"]
mod tests;
