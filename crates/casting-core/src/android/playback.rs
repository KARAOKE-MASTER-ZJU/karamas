use super::engine_runtime;
use crate::engine::ENGINE_STATE;
use jni::sys::{jboolean, jint, jintArray};
use jni::{JNIEnv, objects::JClass};

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_setDlnaDirect(
    _env: JNIEnv, _class: JClass, enabled: jboolean,
) {
    crate::cast::dlna::settings::set_dlna_direct(enabled != 0);
}

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_getDlnaDirect(
    _env: JNIEnv, _class: JClass,
) -> jboolean {
    crate::cast::dlna::settings::get_dlna_direct() as jboolean
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
    engine_runtime().spawn(async {
        if let Err(e) = crate::trigger_next_song().await {
            log::warn!("切下一首失败: {}", e);
        }
    });
}

// 7b. 控制接口：切歌(上一首)
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_prevSong(_env: JNIEnv, _class: JClass) {
    engine_runtime().spawn(async {
        if let Err(e) = crate::trigger_prev_song().await {
            log::warn!("切上一首失败: {}", e);
        }
    });
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
