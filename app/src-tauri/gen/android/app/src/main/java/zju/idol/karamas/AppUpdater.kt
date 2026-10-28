package zju.idol.karamas

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import androidx.core.content.pm.PackageInfoCompat
import app.tauri.annotation.InvokeArg
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import java.io.File

// 独立 provider，避免与分享插件的 FileProvider 合并配置。
class UpdateFileProvider : FileProvider()

@InvokeArg
class InstallUpdateArgs {
    var path: String = ""
    var versionCode: Long = 0
}

// 只负责读取系统版本和唤起安装器，检查与下载由 Rust 处理。
class AppUpdater(private val activity: Activity) {
    @Suppress("DEPRECATION")
    fun version(invoke: Invoke) {
        try {
            val info = activity.packageManager.getPackageInfo(activity.packageName, 0)
            invoke.resolve(JSObject().put("versionCode", PackageInfoCompat.getLongVersionCode(info)))
        } catch (error: Exception) {
            invoke.reject("无法读取应用版本：${error.message}")
        }
    }

    @Suppress("DEPRECATION")
    fun install(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(InstallUpdateArgs::class.java)
            val file = File(args.path).canonicalFile
            val expected = File(activity.cacheDir, "updates/pending.apk").canonicalFile
            check(file == expected && file.isFile) { "安装包已失效，请重新下载" }
            val manager = activity.packageManager
            val info = manager.getPackageArchiveInfo(file.path, 0)
                ?: error("无法读取安装包")
            val installed = manager.getPackageInfo(activity.packageName, 0)
            check(info.packageName == activity.packageName &&
                PackageInfoCompat.getLongVersionCode(info) == args.versionCode &&
                args.versionCode >= PackageInfoCompat.getLongVersionCode(installed)) { "安装包版本不匹配，请重新检查更新" }
            if (Build.VERSION.SDK_INT >= 26 && !manager.canRequestPackageInstalls()) {
                activity.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:${activity.packageName}")))
                invoke.reject("请允许安装此来源的应用，返回后再次点击安装。", "INSTALL_PERMISSION_REQUIRED")
                return
            }
            val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.updates", file)
            activity.startActivity(Intent(Intent.ACTION_VIEW)
                .setDataAndType(uri, "application/vnd.android.package-archive")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
            invoke.resolve()
        } catch (error: Exception) {
            invoke.reject("无法安装更新：${error.message}", "ANDROID_UPDATE")
        }
    }
}
