use super::{bilibili::current_bilibili_session, engine_runtime};
use jni::{
    JNIEnv,
    objects::{JClass, JString},
};
use log::info;

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
        crate::cast::bilibili::init_session_dir(&dir_str);
        if let Err(error) = crate::media::session::init_cache_dir(&dir_str) {
            log::error!(target: "DLNA1080", "初始化 App 媒体缓存目录失败: {}", error);
        }
        info!("Session 目录初始化: {}", dir_str);
    }
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

        let handle = engine_runtime().handle().clone();
        handle.block_on(async {
            // 使用 match 代替 expect，防止 panic 污染全局锁
            if let Err(e) =
                crate::start_engine_core(base_url_str, room_id_str, loc_str, handle.clone()).await
            {
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

/// 启动B站投屏引擎。
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_startBilibiliEngine(
    mut env: JNIEnv,
    _class: JClass,
    base_url: JString,
    room_id: JString,
    device_buvid: JString,
    sender_buvid: JString,
) {
    let base_url_str: String = env.get_string(&base_url).unwrap().into();
    let room_id_str: String = env.get_string(&room_id).unwrap().into();
    let buvid_str: String = env.get_string(&device_buvid).unwrap().into();
    let sender_buvid: String = env.get_string(&sender_buvid).unwrap().into();

    let session = current_bilibili_session();
    let Some(session) = session else {
        log::error!("startBilibiliEngine: not logged in");
        return;
    };

    std::thread::spawn(move || {
        crate::reset_engine();
        let handle = engine_runtime().handle().clone();
        handle.block_on(async {
            if let Err(e) = crate::start_bilibili_engine_core(
                base_url_str,
                room_id_str,
                session,
                buvid_str,
                sender_buvid,
                handle.clone(),
            )
            .await
            {
                log::error!("startBilibiliEngine failed: {}", e);
            }
        });
    });
}
