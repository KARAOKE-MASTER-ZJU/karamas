package zju.idol.karamas

import android.app.Activity
import android.content.Intent
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.ResultReceiver
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONObject

@InvokeArg
class StartCastingArgs {
    var roomId: String = ""
}

@TauriPlugin(
    permissions = [
        Permission(strings = ["android.permission.ACCESS_LOCAL_NETWORK"], alias = "localNetwork"),
        Permission(strings = ["android.permission.POST_NOTIFICATIONS"], alias = "notifications")
    ]
)
class CastingPlatformPlugin(private val activity: Activity) : Plugin(activity) {
    private data class PendingPermission(val alias: String, val invoke: Invoke)

    private val pendingPermissions = mutableListOf<PendingPermission>()
    private val startingServices = mutableSetOf<Invoke>()

    private fun hasLocalNetworkAccess(): Boolean =
        Build.VERSION.SDK_INT < 37 || activity.applicationInfo.targetSdkVersion < 37 ||
            getPermissionState("localNetwork") == PermissionState.GRANTED

    @Command
    fun ensureAccess(invoke: Invoke) {
        if (hasLocalNetworkAccess()) resolveNetwork(invoke)
        else enqueuePermission("localNetwork", invoke)
    }

    @Command
    fun startCasting(invoke: Invoke) {
        val permission = getPermissionState("notifications")
        // 可选权限只在首次使用时询问，拒绝后继续投屏。
        if (Build.VERSION.SDK_INT >= 33 && permission == PermissionState.PROMPT) {
            enqueuePermission("notifications", invoke)
        } else {
            startService(invoke)
        }
    }

    // Tauri 只保存一个权限回调：同类申请合并，不同权限依次申请。
    private fun enqueuePermission(alias: String, invoke: Invoke) {
        pendingPermissions.add(PendingPermission(alias, invoke))
        if (pendingPermissions.size == 1) requestNextPermission()
    }

    private fun requestNextPermission() {
        val request = pendingPermissions.firstOrNull() ?: return
        try {
            requestPermissionForAlias(request.alias, request.invoke, "permissionResult")
        } catch (error: Exception) {
            takePermissionRequests(request.alias).forEach {
                it.invoke.reject("无法申请权限：${error.message}")
            }
            requestNextPermission()
        }
    }

    @PermissionCallback
    private fun permissionResult(@Suppress("UNUSED_PARAMETER") invoke: Invoke) {
        val alias = pendingPermissions.firstOrNull()?.alias ?: return
        takePermissionRequests(alias).forEach {
            if (alias == "notifications") {
                // Android 允许拒绝通知权限后启动前台服务。
                startService(it.invoke)
            } else if (hasLocalNetworkAccess()) {
                resolveNetwork(it.invoke)
            } else {
                it.invoke.reject("请允许访问本地网络，才能搜索和连接投屏设备。", "LOCAL_NETWORK_DENIED")
            }
        }
        requestNextPermission()
    }

    private fun takePermissionRequests(alias: String): List<PendingPermission> {
        val requests = pendingPermissions.filter { it.alias == alias }
        pendingPermissions.removeAll { it.alias == alias }
        return requests
    }

    private fun startService(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(StartCastingArgs::class.java)
            val receiver = object : ResultReceiver(Handler(Looper.getMainLooper())) {
                override fun onReceiveResult(resultCode: Int, resultData: Bundle?) {
                    if (!startingServices.remove(invoke)) return
                    if (resultCode == 0) invoke.resolve()
                    else invoke.reject(resultData?.getString("error") ?: "无法启动后台投屏服务", "ANDROID_SERVICE")
                }
            }
            startingServices.add(invoke)
            val intent = Intent(activity, CastingService::class.java)
                .putExtra(CastingService.EXTRA_ROOM, args.roomId)
                .putExtra(CastingService.EXTRA_RESULT, receiver)
            ContextCompat.startForegroundService(activity, intent)
        } catch (error: Exception) {
            startingServices.remove(invoke)
            invoke.reject("无法启动后台投屏服务：${error.message}", "ANDROID_SERVICE")
        }
    }

    @Command
    fun stopCasting(invoke: Invoke) {
        activity.stopService(Intent(activity, CastingService::class.java))
        invoke.resolve()
    }

    private fun resolveNetwork(invoke: Invoke) {
        val result = JSObject()
        try {
            val manager = activity.getSystemService(ConnectivityManager::class.java)
            val network = manager.activeNetwork
            val capabilities = manager.getNetworkCapabilities(network)
            val properties = manager.getLinkProperties(network)
            result.put("wifi", capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true)
            result.put("vpn", capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true)
            result.put("interfaceName", properties?.interfaceName ?: JSONObject.NULL)
            result.put("addresses", JSArray(properties?.linkAddresses?.map { it.toString() } ?: emptyList<String>()))
            invoke.resolve(result)
        } catch (error: Exception) {
            invoke.reject("无法读取网络状态：${error.message}")
        }
    }

    override fun onDestroy(activity: AppCompatActivity) {
        val requests = pendingPermissions.map { it.invoke } + startingServices
        pendingPermissions.clear()
        startingServices.clear()
        requests.forEach {
            it.reject("页面已关闭，操作已取消，请重试。", "ANDROID_CANCELLED")
        }
    }
}
