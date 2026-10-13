# DLNA 原始链接直连

默认仍通过手机媒体代理投屏。在 Android 设置中开启“DLNA 原始链接直连”后，接收设备会直接请求原始媒体地址。设置持久保存，下次切歌或重新投屏生效。

B站在此模式下使用 720P 单文件 MP4，1080P 的音视频混流不可用。开启直连会将 DLNA 清晰度设为 720P；需要混流的媒体应使用默认代理模式。其他 HTTP(S) 媒体地址及其签名参数保持原样交给设备。

接收设备需要能访问互联网，并支持源站的协议和鉴权要求。直连不会代设备补充 Referer 等请求头，遇到源站拒绝、地址过期或设备不支持 HTTPS 时，请关闭直连并重新投屏。

Rust API：`set_dlna_direct(bool)` / `get_dlna_direct()`。Android JNI：`RustEngine.setDlnaDirect(Boolean)` / `getDlnaDirect()`。
