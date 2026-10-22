use crate::engine::ENGINE_STATE;
use jni::sys::{jint, jstring};
use jni::{
    JNIEnv,
    objects::{JClass, JString},
};

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

/// Returns the last synchronized current-song ID, or an empty string when
/// disconnected or no song exists. Reads local state without network requests.
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getCurrentSongId(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let ctx = ENGINE_STATE.read().ok().and_then(|state| state.as_ref().cloned());
    let id = match ctx {
        Some(ctx) => ctx.rt.block_on(crate::get_current_song_id_core()),
        None => String::new(),
    };
    env.new_string(id).expect("Couldn't create java string!").into_raw()
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

/// Call from Dispatchers.IO. Returns the parts object as JSON, or an empty
/// string on failure, with details in the Rust log.
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getCurrentBilibiliParts(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let ctx = ENGINE_STATE
        .read()
        .ok()
        .and_then(|state| state.as_ref().cloned());
    let json = match ctx {
        Some(ctx) => match ctx.rt.block_on(crate::get_current_bilibili_parts_core()) {
            Ok(parts) => serde_json::to_string(&parts).unwrap_or_default(),
            Err(error) => {
                log::warn!("查询分 P 失败 [{}]: {}", error.code, error);
                String::new()
            }
        },
        None => String::new(),
    };
    env.new_string(json)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// Call from Dispatchers.IO. 1 = room edit accepted (playback follows through
/// room sync), 0 = unchanged, -1 = failed. `page_index` is zero-based.
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_selectCurrentBilibiliPart(
    mut env: JNIEnv,
    _class: JClass,
    song_id: JString,
    page_index: jint,
) -> jint {
    if page_index < 0 {
        return -1;
    }
    let song_id: String = match env.get_string(&song_id) {
        Ok(id) => id.into(),
        Err(error) => {
            log::warn!("读取分 P 选择的歌曲 ID 失败: {}", error);
            return -1;
        }
    };
    let ctx = ENGINE_STATE
        .read()
        .ok()
        .and_then(|state| state.as_ref().cloned());
    let Some(ctx) = ctx else {
        return -1;
    };
    match ctx.rt.block_on(crate::select_current_bilibili_part_core(
        &song_id,
        page_index as u32,
    )) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(error) => {
            log::warn!("选择分 P 失败 [{}]: {}", error.code, error);
            -1
        }
    }
}
