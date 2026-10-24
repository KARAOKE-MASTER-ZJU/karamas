package zju.idol.karamas

import android.app.Activity
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Build
import androidx.appcompat.app.AppCompatActivity
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONObject

@TauriPlugin(
    permissions = [Permission(
        strings = ["android.permission.ACCESS_LOCAL_NETWORK"],
        alias = "localNetwork"
    )]
)
class CastingNetworkPlugin(private val activity: Activity) : Plugin(activity) {
    private val pendingAccessRequests = mutableListOf<Invoke>()

    private fun hasLocalNetworkAccess(): Boolean =
        Build.VERSION.SDK_INT < 37 || activity.applicationInfo.targetSdkVersion < 37 ||
            getPermissionState("localNetwork") == PermissionState.GRANTED

    @Command
    fun ensureAccess(invoke: Invoke) {
        // Tauri 只保存一个权限回调，并发调用共用这次申请。
        if (pendingAccessRequests.isNotEmpty()) {
            pendingAccessRequests.add(invoke)
            return
        }
        if (hasLocalNetworkAccess()) {
            resolveNetwork(invoke)
            return
        }
        pendingAccessRequests.add(invoke)
        try {
            requestPermissionForAlias("localNetwork", invoke, "networkPermissionResult")
        } catch (error: Exception) {
            takePendingAccessRequests().forEach {
                it.reject("无法申请本地网络权限：${error.message}")
            }
        }
    }

    @PermissionCallback
    private fun networkPermissionResult(@Suppress("UNUSED_PARAMETER") invoke: Invoke) {
        val requests = takePendingAccessRequests()
        val granted = hasLocalNetworkAccess()
        requests.forEach {
            if (granted) {
                resolveNetwork(it)
            } else {
                it.reject("请允许访问本地网络，才能搜索和连接投屏设备。", "LOCAL_NETWORK_DENIED")
            }
        }
    }

    private fun takePendingAccessRequests(): List<Invoke> {
        val requests = pendingAccessRequests.toList()
        pendingAccessRequests.clear()
        return requests
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
        takePendingAccessRequests().forEach {
            it.reject("页面已关闭，本地网络授权已取消，请重试。", "LOCAL_NETWORK_CANCELLED")
        }
    }
}
