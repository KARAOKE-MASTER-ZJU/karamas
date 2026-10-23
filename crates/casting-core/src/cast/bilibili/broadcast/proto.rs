//! 广播协议使用的 protobuf 字段子集，未知字段由 prost 跳过。
//! 字段编号来自 bilitv-cloud-progress-go 附带的 bilibili/broadcast、metadata 和 rpc 定义。
//! 手写 Message 派生避免给桌面和 Android 构建增加 protoc 要求。

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct BroadcastFrame {
    #[prost(message, optional, tag = "1")]
    pub options: Option<FrameOption>,
    #[prost(string, tag = "2")]
    pub target_path: String,
    #[prost(message, optional, tag = "3")]
    pub body: Option<prost_types::Any>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct FrameOption {
    #[prost(int64, tag = "1")]
    pub message_id: i64,
    #[prost(int64, tag = "2")]
    pub sequence: i64,
    #[prost(bool, tag = "3")]
    pub is_ack: bool,
    #[prost(message, optional, tag = "4")]
    pub status: Option<RpcStatus>,
    #[prost(string, tag = "5")]
    pub ack_origin: String,
    #[prost(int64, tag = "6")]
    pub timestamp: i64,
    #[prost(int64, tag = "7")]
    pub msg_type: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct RpcStatus {
    #[prost(int32, tag = "1")]
    pub code: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct AuthReq {
    #[prost(string, tag = "1")]
    pub guid: String,
    #[prost(string, tag = "2")]
    pub conn_id: String,
    #[prost(int64, tag = "3")]
    pub last_msg_id: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct TargetPath {
    #[prost(string, repeated, tag = "1")]
    pub target_paths: Vec<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct MessageAckReq {
    #[prost(int64, tag = "1")]
    pub ack_id: i64,
    #[prost(string, tag = "2")]
    pub ack_origin: String,
    #[prost(string, tag = "3")]
    pub target_path: String,
    #[prost(int64, tag = "4")]
    pub msg_type: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct HeartbeatReq {}

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct ProjReply {
    #[prost(int64, tag = "1")]
    pub cmd_type: i64,
    #[prost(int64, tag = "2")]
    pub mid: i64,
    #[prost(int64, tag = "3")]
    pub aid: i64,
    #[prost(int64, tag = "4")]
    pub cid: i64,
    #[prost(int64, tag = "8")]
    pub seek_ts: i64,
    #[prost(string, tag = "9")]
    pub extra: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct Device {
    #[prost(int32, tag = "2")]
    pub build: i32,
    #[prost(string, tag = "3")]
    pub buvid: String,
    #[prost(string, tag = "4")]
    pub mobi_app: String,
    #[prost(string, tag = "5")]
    pub platform: String,
    #[prost(string, tag = "6")]
    pub device: String,
    #[prost(string, tag = "7")]
    pub channel: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct Metadata {
    #[prost(string, tag = "1")]
    pub access_key: String,
    #[prost(string, tag = "2")]
    pub mobi_app: String,
    #[prost(string, tag = "3")]
    pub device: String,
    #[prost(int32, tag = "4")]
    pub build: i32,
    #[prost(string, tag = "5")]
    pub channel: String,
    #[prost(string, tag = "6")]
    pub buvid: String,
    #[prost(string, tag = "7")]
    pub platform: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct Locale {
    #[prost(string, tag = "4")]
    pub timezone: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct Network {
    #[prost(int32, tag = "1")]
    pub r#type: i32,
}
