buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("com.android.tools.build:gradle:9.3.1")
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.2.10")
    }
}

allprojects {
    repositories {
        google()
        mavenCentral()
    }
}

// 分享插件默认使用 SDK 35，需对齐当前应用以满足 Tauri 的依赖要求。
subprojects {
    if (name == "tauri-plugin-sharekit") {
        plugins.withId("com.android.library") {
            extensions.configure<com.android.build.api.variant.LibraryAndroidComponentsExtension>("androidComponents") {
                finalizeDsl { extension ->
                    extension.compileSdk = 37
                }
            }
        }
    }
}

tasks.register("clean").configure {
    delete("build")
}

