# 保留通过反射调用的平台插件和参数类型。
-keep class zju.idol.karamas.CastingPlatformPlugin { *; }
-keep class zju.idol.karamas.StartCastingArgs { *; }

# 保留 JNI 入口和证书验证器通过 JNI 调用的类型。
-keep class zju.idol.karamas.MainActivity {
    private native void initializePlatformVerifier(android.content.Context);
}
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }
