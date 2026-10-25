import java.util.Properties
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

android {
    compileSdk = 37
    // 与 Android 启动脚本共用 NDK，确保 Gradle 能找到原生符号剥离工具。
    System.getenv("NDK_HOME")?.let { ndkHome ->
        ndkPath = ndkHome
        val ndkProperties = Properties().apply {
            file("$ndkHome/source.properties").inputStream().use { load(it) }
        }
        ndkVersion = ndkProperties.getProperty("Pkg.Revision")
    }
    namespace = "zju.idol.karamas"
    // 直接引用由源 SVG 转换的矢量 XML 图标，各屏幕密度共用一份。
    sourceSets.getByName("main").res.srcDir("../../../icons/android")
    defaultConfig {
        // DLNA 描述文件、控制请求和本地媒体代理使用 HTTP。
        manifestPlaceholders["usesCleartextTraffic"] = "true"
        applicationId = "zju.idol.karamas"
        minSdk = 24
        targetSdk = 37
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
        }
        getByName("release") {
            optimization {
               enable = true
            }
            proguardFiles(
                *fileTree(".") {
                  include("**/*.pro")
                  exclude("build/**")
                }.files.toTypedArray()
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    buildFeatures {
        buildConfig = true
    }
    packaging {
        jniLibs {
            // 按需压缩原生库，默认保留直接从 APK 加载的方式。
            useLegacyPackaging = providers.gradleProperty("karamasCompressNativeLibs")
                .map { it.toBooleanStrict() }
                .orElse(false)
                .get()
        }
    }
}

// 只为 debug 变体保留原生符号，避免全局 packaging 配置影响 release。
androidComponents {
    onVariants(selector().withBuildType("debug")) { variant ->
        variant.packaging.jniLibs.keepDebugSymbols.add("**/*.so")
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
}

apply(from = file("tauri.build.gradle.kts"))
