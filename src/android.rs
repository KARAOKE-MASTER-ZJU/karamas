#[allow(non_snake_case)]
use crate::ENGINE_STATE;
use jni::JNIEnv;
use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jint, jobjectArray, jsize, jstring, jintArray};
use jni::JavaVM;
use log::{info, Log, Metadata, Record};
use std::sync::OnceLock;

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_setDlnaDirect(
    _env: JNIEnv, _class: JClass, enabled: jboolean,
) {
    crate::set_dlna_direct(enabled != 0);
}

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getDlnaDirect(
    _env: JNIEnv, _class: JClass,
) -> jboolean {
    crate::get_dlna_direct() as jboolean
}

// 1. 日志初始化
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_initLogging(
    mut env: JNIEnv,
    _class: JClass,
    level: jint,
) {
    let log_level = match level {
        0 => log::LevelFilter::Error,
        1 => log::LevelFilter::Warn,
        2 => log::LevelFilter::Info,
        _ => log::LevelFilter::Debug,
    };

    let _ = init_jni_log_bridge(&mut env);

    // 注册我们的 JNI logger（将 Rust 日志转发到 Java），这是全局唯一的 logger
    let _ = log::set_boxed_logger(Box::new(JniLogger::new()));
    log::set_max_level(log_level);

    // 顺便把 Crypto 也初始化
    let _ = rustls::crypto::ring::default_provider().install_default();

    info!("Android 日志与 Crypto 模块初始化完成");
}

/// 初始化 Session 存储目录（应在应用启动时调用，传入 context.filesDir.absolutePath）
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_initSessionDir(
    mut env: JNIEnv,
    _class: JClass,
    dir_path: JString,
) {
    let dir_str = match env.get_string(&dir_path) {
        Ok(s) => s.into(),
        Err(_) => String::new(),
    };
    if !dir_str.is_empty() {
        crate::cast::bilibili_caster::init_session_dir(&dir_str);
        if let Err(error) = crate::media_session::init_cache_dir(&dir_str) {
            log::error!(target: "DLNA1080", "初始化 App 媒体缓存目录失败: {}", error);
        }
        info!("Session 目录初始化: {}", dir_str);
    }
}

fn set_bili_auth_state(next: crate::BiliAuthState) {
    if let Ok(mut state) = crate::BILI_AUTH_STATE.write() {
        *state = next;
    }
}

fn reset_bili_auth_state() {
    set_bili_auth_state(crate::BiliAuthState::Idle);
}

fn current_bilibili_session() -> Option<crate::cast::bilibili_caster::BilibiliSession> {
    use crate::{BILI_AUTH_STATE, BiliAuthState};

    let state = BILI_AUTH_STATE.read().ok()?;
    match &*state {
        BiliAuthState::Success(session) => Some(session.clone()),
        _ => None,
    }
}

struct LogBridge {
    java_vm: JavaVM,
    rust_engine_class: GlobalRef,
}

static LOG_BRIDGE: OnceLock<LogBridge> = OnceLock::new();

fn init_jni_log_bridge(env: &mut JNIEnv) -> Result<(), String> {
    if LOG_BRIDGE.get().is_some() {
        return Ok(());
    }

    let java_vm = env.get_java_vm().map_err(|e| format!("get_java_vm failed: {e}"))?;
    let rust_engine_local = env
        .find_class("zju/bangdream/ktv/casting/RustEngine")
        .map_err(|e| format!("find_class failed: {e}"))?;
    let rust_engine_class = env
        .new_global_ref(rust_engine_local)
        .map_err(|e| format!("new_global_ref failed: {e}"))?;

    LOG_BRIDGE
        .set(LogBridge {
            java_vm,
            rust_engine_class,
        })
        .map_err(|_| "LOG_BRIDGE already initialized".to_string())?;

    Ok(())
}

struct JniLogger {
    // minimal; 直接把日志转发到 Java，必要时也可以扩展把日志写到其他地方
}

impl JniLogger {
    fn new() -> Self {
        Self {}
    }
}

impl Log for JniLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // 开发时把所有级别都允许，让 log::set_max_level 控制最终输出
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // 把日志也打印到 stderr（便于非 Android 平台和调试）
        eprintln!("[{}] {}: {}", record.level(), record.target(), record.args());

        let Some(bridge) = LOG_BRIDGE.get() else { return; };
        let level = match record.level() {
            log::Level::Trace => 0,
            log::Level::Debug => 1,
            log::Level::Info => 2,
            log::Level::Warn => 3,
            log::Level::Error => 4,
        };
        let target = record.target();
        let message = format!("{}", record.args());
        if let Ok(mut env) = bridge.java_vm.attach_current_thread() {
            let Ok(target_j) = env.new_string(target) else { return; };
            let Ok(message_j) = env.new_string(message) else { return; };

            // 从 GlobalRef 借用 JClass 描述符（避免临时对象转换错误）
            let cls = <&JClass>::from(bridge.rust_engine_class.as_obj());
            let _ = env.call_static_method(
                cls,
                "onRustLog",
                "(ILjava/lang/String;Ljava/lang/String;)V",
                &[
                    JValue::from(level as jint),
                    JValue::from(&target_j),
                    JValue::from(&message_j),
                ],
            );
        }
    }

    fn flush(&self) {
        // no-op: 当前 logger 不维护额外的缓冲区
    }
}

// 2. 搜索接口
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_searchDevices(
    mut env: JNIEnv,
    _class: JClass,
) -> jobjectArray {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dlna_devices = rt.block_on(crate::discover_devices_core());

    let cls = env
        .find_class("zju/bangdream/ktv/casting/DlnaDeviceItem")
        .unwrap();
    let array = env
        .new_object_array(dlna_devices.len() as jsize, &cls, JObject::null())
        .unwrap();
    for (i, d) in dlna_devices.iter().enumerate() {
        let name = env.new_string(&d.friendly_name).unwrap();
        let loc = env.new_string(&d.location).unwrap();
        let item = env
            .new_object(
                &cls,
                "(Ljava/lang/String;Ljava/lang/String;)V",
                &[(&name).into(), (&loc).into()],
            )
            .unwrap();
        env.set_object_array_element(&array, i as jsize, item)
            .unwrap();
    }
    array.into_raw()
}

// 3. 核心初始化接口 (支持重复调用以更换设备)
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_startEngine(
    mut env: JNIEnv,
    _class: JClass,
    base_url: JString,
    room_id: JString,
    target_location: JString,
) {
    let base_url_str: String = env.get_string(&base_url).unwrap().into();
    let loc_str: String = env.get_string(&target_location).unwrap().into();
    let room_id_str: String = env.get_string(&room_id).unwrap().into();

    std::thread::spawn(move || {
        // 启动前先尝试清理旧状态，防止冲突
        crate::reset_engine();

        // 创建 Runtime
        let rt = tokio::runtime::Runtime::new().unwrap();

        // 在这个 Runtime 里跑异步初始化
        // 我们需要把 rt 的所有权传给 start_engine_core，所以这里用 handle 来 block_on
        let handle = rt.handle().clone();
        handle.block_on(async {
            // 使用 match 代替 expect，防止 panic 污染全局锁
            if let Err(e) = crate::start_engine_core(base_url_str, room_id_str, loc_str, rt).await {
                log::error!("Failed to start engine: {}", e);
            }
        });
    });
}

// 4. 接口：重置引擎 (UI 点击重新选择设备时调用)
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_resetEngine(
    _env: JNIEnv,
    _class: JClass,
) {
    crate::reset_engine();
}

// 5. 数据接口：获取当前播放进度
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_queryProgress(
    env: JNIEnv,
    _class: JClass,
) -> jintArray {
    let (current, total) = if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            ctx.rt.block_on(crate::get_current_progress())
        } else {
            (-1, -1)
        }
    } else {
        (-1, -1)
    };

    let result_array = env.new_int_array(2).expect("无法创建 Java 数组");

    // 将 Rust 的数据放入数组
    let data = [current as jint, total as jint];
    env.set_int_array_region(&result_array, 0, &data).expect("无法填充数组数据");

    // 返回 Java 数组引用
    result_array.into_raw()
}

// 自动轮询使用独立入口，避免将手动 nextSong 也限制为 once。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_pollPlaybackProgress(
    env: JNIEnv,
    _class: JClass,
) -> jintArray {
    let ctx = ENGINE_STATE.read().ok().and_then(|guard| guard.as_ref().cloned());
    let (current, total) = match ctx {
        Some(ctx) => ctx.rt.block_on(crate::poll_playback_progress()),
        None => (-1, -1),
    };
    let result = env.new_int_array(2).expect("无法创建 Java 数组");
    env.set_int_array_region(&result, 0, &[current, total]).expect("无法填充数组数据");
    result.into_raw()
}

// 7a. 控制接口：切歌(下一首)
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_nextSong(_env: JNIEnv, _class: JClass) {
    crate::trigger_next_song();
}
// 7b. 控制接口：切歌(上一首)
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_prevSong(_env: JNIEnv, _class: JClass) {
    crate::trigger_prev_song();
}

/// Called from Kotlin's IO dispatcher; no engine initialization or playlist mutation.
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_recastCurrentSong(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    let ctx = ENGINE_STATE.read().ok().and_then(|guard| guard.as_ref().cloned());
    match ctx {
        Some(ctx) => ctx.rt.block_on(crate::recast_current_song(&ctx)),
        None => 0,
    }
}

// 8. 控制接口：播放/暂停 切换
// 返回 1 表示播放，0 表示暂停，失败返回 -1
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_togglePause(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            return match ctx.rt.block_on(crate::toggle_pause_core()) {
                Ok(is_playing) => {
                    if is_playing {
                        1
                    } else {
                        0
                    }
                }
                Err(_) => -1,
            };
        }
    }
    -1
}

// 9. 控制接口：音量调节
// 返回设置后的音量，失败返回 -1
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_setVolume(
    _env: JNIEnv,
    _class: JClass,
    volume: jint,
) -> jint {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            return match ctx.rt.block_on(crate::set_volume_core(volume as u32)) {
                Ok(v) => v as jint,
                Err(_) => -1,
            };
        }
    }
    -1
}

// 10. 控制接口：获取当前音量
// 返回当前音量，失败返回 -1
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getVolume(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            return match ctx.rt.block_on(crate::get_volume_core()) {
                Ok(v) => v as jint,
                Err(_) => -1,
            };
        }
    }
    -1
}


// 10b. 控制接口：相对音量调节（B站投屏只支持相对步进，DLNA 内部按 get+set 模拟）
// 返回 1 表示成功，-1 表示失败
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_volumeUp(
    _env: JNIEnv,
    _class: JClass,
    step: jint,
) -> jint {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            return match ctx.rt.block_on(crate::volume_up_core(step as u32)) {
                Ok(()) => 1,
                Err(_) => -1,
            };
        }
    }
    -1
}

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_volumeDown(
    _env: JNIEnv,
    _class: JClass,
    step: jint,
) -> jint {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            return match ctx.rt.block_on(crate::volume_down_core(step as u32)) {
                Ok(()) => 1,
                Err(_) => -1,
            };
        }
    }
    -1
}

// 10c. 控制接口：弹幕开关（仅B站投屏支持，DLNA 会返回 -1）
// 返回切换后的状态：1=开，0=关，-1=失败
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_toggleDanmaku(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            return match ctx.rt.block_on(crate::toggle_danmaku_core()) {
                Ok(true) => 1,
                Ok(false) => 0,
                Err(_) => -1,
            };
        }
    }
    -1
}

// 直接设置弹幕开关（供开关类 UI 使用，避免依赖本地状态做盲翻转）
// 返回设置后的状态：1=开，0=关，-1=失败
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_setDanmaku(
    _env: JNIEnv,
    _class: JClass,
    on: jboolean,
) -> jint {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            return match ctx.rt.block_on(crate::set_danmaku_core(on != 0)) {
                Ok(true) => 1,
                Ok(false) => 0,
                Err(_) => -1,
            };
        }
    }
    -1
}

// 返回本地跟踪的弹幕状态（设备侧无读回接口），1=开，0=关
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getDanmakuState(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    jboolean::from(crate::get_danmaku_core())
}

// 10d. 控制接口：清晰度（B站支持全部档位，DLNA 支持 720/1080 beta）
// qn 跨 JNI 边界仍传裸 int（B站协议常量：16/32/64/80/116），Rust 内部转换成
// Quality enum 做合法性校验，拒绝非法档位。
// 返回设置后的 qn，失败（含非法 qn）返回 -1
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_setQuality(
    _env: JNIEnv,
    _class: JClass,
    qn: jint,
) -> jint {
    let Some(quality) = crate::cast::Quality::from_qn(qn as u32) else {
        return -1;
    };
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            return match ctx.rt.block_on(crate::set_quality_core(quality)) {
                Ok(q) => q.as_qn() as jint,
                Err(_) => -1,
            };
        }
    }
    -1
}

// 返回本地跟踪的清晰度 qn（设备侧无读回接口）
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getQuality(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    crate::get_quality_core().as_qn() as jint
}

// 11. 控制接口：跳转进度 (Seek)
// 返回 1 表示成功，-1 表示失败
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_jumpToSecs(
    _env: JNIEnv,
    _class: JClass,
    target_secs: jint,
) -> jint {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            return match ctx.rt.block_on(crate::jump_to_secs(target_secs as u32)) {
                Ok(_) => 1,
                Err(_) => -1
            };
        }
    }
    -1
}

// 13. 搜索接口：通过设备描述XML的URL直接获取设备（适用于WiFi不支持多播的场景）
// 例如：http://192.168.1.x:9958/bilibili/description.xml（B站小电视默认地址）
// 成功返回包含一个元素的设备数组，失败返回空数组
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_searchDeviceByUrl(
    mut env: JNIEnv,
    _class: JClass,
    url: JString,
) -> jobjectArray {
    let url_str: String = env.get_string(&url).unwrap().into();
    let rt = tokio::runtime::Runtime::new().unwrap();

    let cls = env
        .find_class("zju/bangdream/ktv/casting/DlnaDeviceItem")
        .unwrap();

    match rt.block_on(crate::discover_device_from_url_core(url_str)) {
        Ok(device) => {
            let array = env
                .new_object_array(1, &cls, JObject::null())
                .unwrap();
            let name = env.new_string(&device.friendly_name).unwrap();
            let loc = env.new_string(&device.location).unwrap();
            let item = env
                .new_object(
                    &cls,
                    "(Ljava/lang/String;Ljava/lang/String;)V",
                    &[(&name).into(), (&loc).into()],
                )
                .unwrap();
            env.set_object_array_element(&array, 0, item).unwrap();
            array.into_raw()
        }
        Err(e) => {
            info!("通过URL搜索设备失败: {}", e);
            env.new_object_array(0, &cls, JObject::null())
                .unwrap()
                .into_raw()
        }
    }
}

// ---- Bilibili 登录 & 投屏 ----

/// 启动B站扫码登录（异步后台）。登录状态通过 getBilibiliAuthState() 查询。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_startBilibiliQrLogin(
    _env: JNIEnv,
    _class: JClass,
) {
    use crate::cast::bilibili_caster::login_qr;
    use crate::BiliAuthState;

    reset_bili_auth_state();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let result = login_qr(|qr_url| {
                set_bili_auth_state(BiliAuthState::AwaitingScan { qr_url });
            })
            .await;
            match result {
                Ok(session) => set_bili_auth_state(BiliAuthState::Success(session)),
                Err(e) => set_bili_auth_state(BiliAuthState::Error(e)),
            }
        });
    });
}

/// 返回当前的扫码 URL（可用于在 Android 侧渲染二维码）。
/// 未就绪时返回空字符串。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getBilibiliQrUrl(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    use crate::{BILI_AUTH_STATE, BiliAuthState};
    let url = if let Ok(s) = BILI_AUTH_STATE.read() {
        match &*s {
            BiliAuthState::AwaitingScan { qr_url } => qr_url.clone(),
            _ => String::new(),
        }
    } else {
        String::new()
    };
    env.new_string(url).unwrap().into_raw()
}

/// 登录状态：0=等待扫码，1=成功，-1=失败/过期，-2=未开始
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getBilibiliLoginStatus(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    use crate::{BILI_AUTH_STATE, BiliAuthState};
    if let Ok(s) = BILI_AUTH_STATE.read() {
        match &*s {
            BiliAuthState::Idle => -2,
            BiliAuthState::AwaitingScan { .. } => 0,
            BiliAuthState::Success(_) => 1,
            BiliAuthState::Error(_) => -1,
        }
    } else {
        -1
    }
}

/// 列出在线的B站投屏设备，返回 JSON 字符串（数组）。
/// 需要已登录（getBilibiliLoginStatus() == 1）。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_listBilibiliDevices(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    use crate::cast::bilibili_caster::{clear_session, list_devices};

    let session = current_bilibili_session();
    let json = if let Some(session) = session {
        let rt = tokio::runtime::Runtime::new().unwrap();
        match rt.block_on(list_devices(&session)) {
            Ok(devs) => serde_json::to_string(&devs).unwrap_or_else(|_| "[]".into()),
            Err(e) => {
                if e.starts_with("auth expired:") {
                    log::warn!("[Bilibili] 设备列表鉴权失效，清理当前会话: {}", e);
                    let _ = clear_session();
                    set_bili_auth_state(crate::BiliAuthState::Error("session expired".into()));
                }
                log::error!("listBilibiliDevices: {}", e);
                "[]".into()
            }
        }
    } else {
        "[]".into()
    };

    env.new_string(json).unwrap().into_raw()
}

/// 检查 B 站 token 是否已过期。返回 true 表示已过期，false 表示有效或无法判断。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_isBilibiliSessionExpired(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    use crate::cast::bilibili_caster::{is_session_expired, load_session};

    if let Some(session) = load_session() {
        if is_session_expired(&session) {
            log::warn!("[Bilibili] Token 已过期，清除 session");
            let _ = crate::cast::bilibili_caster::clear_session();
            set_bili_auth_state(crate::BiliAuthState::Error("session expired".into()));
            return jboolean::from(true);
        }
    }
    jboolean::from(false)
}

/// 清除保存的 B 站 session。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_clearBilibiliSession(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    use crate::cast::bilibili_caster::clear_session;

    match clear_session() {
        Ok(_) => {
            reset_bili_auth_state();
            jboolean::from(true)
        }
        Err(e) => {
            log::error!("[Bilibili] 清除 session 失败: {}", e);
            jboolean::from(false)
        }
    }
}

/// 启动B站投屏引擎。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_startBilibiliEngine(
    mut env: JNIEnv,
    _class: JClass,
    base_url: JString,
    room_id: JString,
    device_buvid: JString,
) {
    let base_url_str: String = env.get_string(&base_url).unwrap().into();
    let room_id_str: String = env.get_string(&room_id).unwrap().into();
    let buvid_str: String = env.get_string(&device_buvid).unwrap().into();

    let session = current_bilibili_session();
    let Some(session) = session else {
        log::error!("startBilibiliEngine: not logged in");
        return;
    };

    std::thread::spawn(move || {
        crate::reset_engine();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.handle().clone();
        handle.block_on(async {
            if let Err(e) = crate::start_bilibili_engine_core(
                base_url_str, room_id_str, session, buvid_str, rt,
            )
            .await
            {
                log::error!("startBilibiliEngine failed: {}", e);
            }
        });
    });
}

/// 返回当前登录 Session 的 JSON 字符串，供 Android 持久化到 EncryptedSharedPreferences。
/// 未登录时返回空字符串。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getBilibiliSessionJson(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let json = current_bilibili_session()
        .and_then(|session| serde_json::to_string(&session).ok())
        .unwrap_or_default();
    env.new_string(json).unwrap().into_raw()
}

/// 从 Android 传入的 JSON 字符串恢复 Session（应用启动时调用）。
/// 成功返回 1，JSON 无效返回 0。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_restoreBilibiliSession(
    mut env: JNIEnv,
    _class: JClass,
    json: JString,
) -> jint {
    use crate::cast::bilibili_caster::{clear_session, is_session_expired, save_session, BilibiliSession};

    let json_str: String = env.get_string(&json).unwrap().into();
    if let Ok(session) = serde_json::from_str::<BilibiliSession>(&json_str) {
        if is_session_expired(&session) {
            log::warn!("[Bilibili] 忽略已过期的恢复会话");
            let _ = clear_session();
            reset_bili_auth_state();
            return 0;
        }
        if let Err(e) = save_session(&session) {
            log::error!("[Bilibili] 恢复会话落盘失败: {}", e);
            reset_bili_auth_state();
            return 0;
        }

        set_bili_auth_state(crate::BiliAuthState::Success(session));
        return 1;
    }

    0
}

// 12. 数据接口：获取当前歌曲标题
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getCurrentSongTitle(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let title = if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            ctx.rt.block_on(async {
                ctx.playlist_manager.get_song_title().await
                    .unwrap_or_else(|| "暂无歌曲".to_string())
            })
        } else {
            "引擎未初始化".to_string()
        }
    } else {
        "系统锁异常".to_string()
    };

    env.new_string(title)
        .expect("Couldn't create java string!")
        .into_raw()
}

// 获取未播歌曲数量
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getQueuedSongsCount(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            return ctx.rt.block_on(async {
                ctx.playlist_manager.get_queued_count().await as jint
            });
        }
    }
    -1
}

// 获取未播歌曲数量
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getSungSongsCount(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    if let Ok(guard) = ENGINE_STATE.read() {
        if let Some(ctx) = guard.as_ref() {
            return ctx.rt.block_on(async {
                ctx.playlist_manager.get_sung_count().await as jint
            });
        }
    }
    -1
}
