# 投屏核心

提供设备发现与控制、点歌台房间同步、媒体解析和本地媒体代理，供应用或服务端调用。

```text
src/
├── lib.rs          # 库入口和公共接口
├── main.rs         # 命令行入口
├── engine/         # 引擎生命周期和播放协调
├── playlist/       # 房间歌单、同步和切歌
│   └── tests/      # 歌单模块测试
├── cast/           # 设备控制接口和协议实现
│   ├── dlna/       # DLNA 设备发现与控制
│   └── bilibili/   # B 站认证、会话和投屏控制
├── sources/        # B 站和 YouTube 媒体解析
├── media/          # 媒体会话、缓存和上游请求
│   ├── mp4/        # MP4 索引与封装
│   └── server/     # HTTP 媒体代理
├── android/        # Android JNI 接入和日志转发
├── cli/            # 命令行交互
├── config.rs       # 共用超时配置
└── error.rs        # 公共错误类型
```
