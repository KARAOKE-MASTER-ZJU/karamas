use jni::EnvUnowned;
use jni::errors::ThrowRuntimeExAndDefault;
use jni::jni_mangle;
use jni::objects::JObject;

#[jni_mangle("zju.idol.karamas.MainActivity")]
pub fn initialize_platform_verifier<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _activity: JObject<'caller>,
    context: JObject<'caller>,
) {
    unowned_env
        .with_env(|env| rustls_platform_verifier::android::init_with_env(env, context))
        .resolve::<ThrowRuntimeExAndDefault>();
}
