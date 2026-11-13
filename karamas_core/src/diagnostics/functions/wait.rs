//! 等待设备状态或播放进度满足自动测试的下一步条件。
use super::{FunctionState, Runner, soap, successful};
use crate::{cast::dlna::parse_time_secs, diagnostics::FunctionCheck};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

impl<F: FnMut(FunctionCheck)> Runner<F> {
    async fn wait_for(
        &mut self,
        title: &str,
        action: &str,
        mut evaluate: impl FnMut(&BTreeMap<String, String>) -> Result<(bool, String), String>,
    ) -> bool {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let endpoint = self
                .services
                .iter()
                .find(|item| item.service_type.contains(":AVTransport:"))
                .unwrap();
            let response = match soap(endpoint, action, &[("InstanceID".into(), "0".into())]).await
            {
                Ok(response) => response,
                Err(error) => {
                    self.publish(
                        title,
                        "AVTransport",
                        action,
                        FunctionState::Failed,
                        error.to_string(),
                        None,
                    );
                    return false;
                }
            };
            if !successful(&response) {
                self.record(title, "AVTransport", action, response);
                return false;
            }
            let (state, detail) = match evaluate(&response.fields) {
                Ok((true, _)) => {
                    self.record(title, "AVTransport", action, response);
                    return true;
                }
                Ok((false, detail)) if Instant::now() >= deadline => {
                    (FunctionState::Failed, format!("{detail}，等待超时"))
                }
                Ok((false, detail)) => (FunctionState::Running, detail),
                Err(detail) => (FunctionState::Failed, detail),
            };
            let finished = matches!(state, FunctionState::Failed);
            self.publish(title, "AVTransport", action, state, detail, Some(response));
            if finished {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    pub(super) async fn wait_state(&mut self, title: &str, expected: &str) -> bool {
        self.wait_for(title, "GetTransportInfo", |fields| {
            let current = fields
                .get("CurrentTransportState")
                .ok_or_else(|| "未返回 CurrentTransportState".to_owned())?
                .trim();
            Ok((
                current == expected,
                format!("当前 {current}，等待 {expected}"),
            ))
        })
        .await
    }

    pub(super) async fn wait_progress(&mut self) -> bool {
        let mut previous = None;
        self.wait_for("播放进度确认", "GetPositionInfo", |fields| {
            let current = fields
                .get("RelTime")
                .ok_or_else(|| "未返回 RelTime".to_owned())?;
            let seconds =
                parse_time_secs(current).ok_or_else(|| format!("无效的 RelTime：{current}"))?;
            let advanced = previous.is_some_and(|previous| seconds > previous);
            previous = Some(seconds);
            Ok((advanced, format!("当前进度 {current}，等待进度推进")))
        })
        .await
    }
}
