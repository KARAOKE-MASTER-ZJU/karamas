use jni::sys::jint;
use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JClass, JValue},
};
use log::{Log, Metadata, Record, info};
use std::sync::OnceLock;

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
