//! 自动执行常用投屏流程，逐项回传结果及实际请求。
mod wait;
use super::{PlaybackTest, PlaybackTester, Service, TestResponse, device_info, upnp::soap};
use crate::{CoreError, cast::MediaFormat};
use serde::Serialize;
use std::collections::BTreeMap;

fn successful(response: &TestResponse) -> bool {
    response
        .status
        .is_some_and(|status| (200..300).contains(&status))
        && response.error.is_none()
        && !response.fields.contains_key("faultcode")
        && !response.fields.contains_key("errorCode")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FunctionState {
    Running,
    Passed,
    Failed,
    Undeclared,
    Skipped,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionCheck {
    pub title: String,
    pub service: String,
    pub action: String,
    pub state: FunctionState,
    pub detail: String,
    pub response: Option<TestResponse>,
}

struct Runner<F> {
    services: Vec<Service>,
    report: F,
}

impl<F: FnMut(FunctionCheck)> Runner<F> {
    fn has_service(&self, name: &str) -> bool {
        self.services
            .iter()
            .any(|service| service.service_type.contains(&format!(":{name}:")))
    }

    fn publish(
        &mut self,
        title: &str,
        service: &str,
        action: &str,
        state: FunctionState,
        detail: String,
        response: Option<TestResponse>,
    ) {
        (self.report)(FunctionCheck {
            title: title.into(),
            service: service.into(),
            action: action.into(),
            state,
            detail,
            response,
        });
    }

    fn record(
        &mut self,
        title: &str,
        service: &str,
        action: &str,
        response: TestResponse,
    ) -> Option<BTreeMap<String, String>> {
        let passed = successful(&response);
        let fields = passed.then(|| response.fields.clone());
        let detail = if passed {
            String::new()
        } else {
            response
                .error
                .clone()
                .or_else(|| response.fields.get("errorDescription").cloned())
                .unwrap_or_else(|| {
                    response
                        .status
                        .map(|status| format!("HTTP {status}"))
                        .unwrap_or_default()
                })
        };
        self.publish(
            title,
            service,
            action,
            if passed {
                FunctionState::Passed
            } else {
                FunctionState::Failed
            },
            detail,
            Some(response),
        );
        fields
    }

    async fn action(
        &mut self,
        title: &str,
        service: &str,
        action: &str,
        args: &[(&str, &str)],
    ) -> Option<BTreeMap<String, String>> {
        let Some(endpoint) = self
            .services
            .iter()
            .find(|item| item.service_type.contains(&format!(":{service}:")))
        else {
            self.publish(
                title,
                service,
                action,
                FunctionState::Undeclared,
                "设备未提供此服务".into(),
                None,
            );
            return None;
        };
        let args = args
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect::<Vec<_>>();
        let response = soap(endpoint, action, &args);
        (self.report)(FunctionCheck {
            title: title.into(),
            service: service.into(),
            action: action.into(),
            state: FunctionState::Running,
            detail: String::new(),
            response: None,
        });
        match response.await {
            Ok(response) => self.record(title, service, action, response),
            Err(error) => {
                self.publish(
                    title,
                    service,
                    action,
                    FunctionState::Failed,
                    error.to_string(),
                    None,
                );
                None
            }
        }
    }

    async fn playback(
        &mut self,
        tester: &PlaybackTester,
        title: &str,
        action: &str,
        test: PlaybackTest,
        runtime: tokio::runtime::Handle,
    ) -> bool {
        if !self.has_service("AVTransport") {
            self.publish(
                title,
                "AVTransport",
                action,
                FunctionState::Undeclared,
                "设备未提供此服务".into(),
                None,
            );
            return false;
        }
        self.publish(
            title,
            "AVTransport",
            action,
            FunctionState::Running,
            String::new(),
            None,
        );
        match tester.execute(test, runtime).await {
            Ok(responses) => {
                let mut passed = true;
                for response in responses {
                    passed &= self
                        .record(title, "AVTransport", action, response)
                        .is_some();
                }
                passed
            }
            Err(error) => {
                self.publish(
                    title,
                    "AVTransport",
                    action,
                    FunctionState::Failed,
                    error.to_string(),
                    None,
                );
                false
            }
        }
    }
}

impl PlaybackTester {
    pub async fn test_functions(
        &self,
        song_url: &str,
        runtime: tokio::runtime::Handle,
        report: impl FnMut(FunctionCheck),
    ) -> Result<(), CoreError> {
        let info = device_info(&self.location).await?;
        let mut runner = Runner {
            services: info.services,
            report,
        };
        if runner
            .record("设备信息", "Device", "description", info.response)
            .is_none()
        {
            return Ok(());
        }
        runner
            .action("媒体协议", "ConnectionManager", "GetProtocolInfo", &[])
            .await;
        for (title, action) in [
            ("播放状态", "GetTransportInfo"),
            ("媒体信息", "GetMediaInfo"),
            ("设备能力", "GetDeviceCapabilities"),
            ("播放设置", "GetTransportSettings"),
            ("可用动作", "GetCurrentTransportActions"),
        ] {
            runner
                .action(title, "AVTransport", action, &[("InstanceID", "0")])
                .await;
        }
        let mut media_ready = false;
        if runner.has_service("AVTransport") {
            runner.publish(
                "设置媒体",
                "AVTransport",
                "SetAVTransportURI",
                FunctionState::Running,
                String::new(),
                None,
            );
            match self
                .execute(
                    PlaybackTest::Load {
                        song_url: song_url.into(),
                        format: MediaFormat::Mp4,
                        direct: false,
                    },
                    runtime.clone(),
                )
                .await
            {
                Ok(responses) => {
                    let played = responses.len() == 3;
                    let mut play_passed = false;
                    for ((title, action), response) in [
                        ("播放前停止", "Stop"),
                        ("设置媒体", "SetAVTransportURI"),
                        ("播放", "Play"),
                    ]
                    .into_iter()
                    .zip(responses)
                    {
                        let passed = runner
                            .record(title, "AVTransport", action, response)
                            .is_some();
                        if action == "Play" {
                            play_passed = passed;
                        }
                    }
                    if !played {
                        runner.publish(
                            "播放",
                            "AVTransport",
                            "Play",
                            FunctionState::Skipped,
                            "设置媒体未成功".into(),
                            None,
                        );
                    }
                    if play_passed {
                        media_ready = runner.wait_state("播放状态确认", "PLAYING").await
                            && runner.wait_progress().await;
                    }
                }
                Err(error) => {
                    runner.publish(
                        "设置媒体",
                        "AVTransport",
                        "SetAVTransportURI",
                        FunctionState::Failed,
                        error.to_string(),
                        None,
                    );
                    runner.publish(
                        "播放",
                        "AVTransport",
                        "Play",
                        FunctionState::Skipped,
                        "测试媒体未就绪".into(),
                        None,
                    );
                }
            }
        } else {
            for (title, action) in [("设置媒体", "SetAVTransportURI"), ("播放", "Play")] {
                runner.publish(
                    title,
                    "AVTransport",
                    action,
                    FunctionState::Undeclared,
                    "设备未提供此服务".into(),
                    None,
                );
            }
        }
        for (title, action, test) in [
            ("暂停", "Pause", PlaybackTest::Pause),
            ("继续", "Play", PlaybackTest::Play),
            ("定位", "Seek", PlaybackTest::Seek { seconds: 0 }),
            ("读取进度", "GetPositionInfo", PlaybackTest::Read),
        ] {
            if !media_ready {
                runner.publish(
                    title,
                    "AVTransport",
                    action,
                    FunctionState::Skipped,
                    "测试媒体未就绪".into(),
                    None,
                );
                continue;
            }
            let passed = runner
                .playback(self, title, action, test, runtime.clone())
                .await;
            if passed {
                match action {
                    "Pause" => {
                        runner.wait_state("暂停状态确认", "PAUSED_PLAYBACK").await;
                    }
                    "Play" => {
                        runner.wait_state("继续状态确认", "PLAYING").await;
                    }
                    _ => {}
                }
            }
        }
        for (label, get, set, field, parameter) in [
            (
                "音量",
                "GetVolume",
                "SetVolume",
                "CurrentVolume",
                "DesiredVolume",
            ),
            ("静音", "GetMute", "SetMute", "CurrentMute", "DesiredMute"),
        ] {
            let fields = runner
                .action(
                    &format!("读取{label}"),
                    "RenderingControl",
                    get,
                    &[("InstanceID", "0"), ("Channel", "Master")],
                )
                .await;
            if let Some(value) = fields.and_then(|mut fields| fields.remove(field)) {
                runner
                    .action(
                        &format!("设置{label}"),
                        "RenderingControl",
                        set,
                        &[
                            ("InstanceID", "0"),
                            ("Channel", "Master"),
                            (parameter, &value),
                        ],
                    )
                    .await;
            } else {
                runner.publish(
                    &format!("设置{label}"),
                    "RenderingControl",
                    set,
                    FunctionState::Skipped,
                    format!("未取得当前{label}"),
                    None,
                );
            }
        }
        let stopped = runner
            .playback(self, "停止", "Stop", PlaybackTest::Stop, runtime)
            .await;
        if stopped {
            runner.wait_state("停止状态确认", "STOPPED").await;
        }
        Ok(())
    }
}
