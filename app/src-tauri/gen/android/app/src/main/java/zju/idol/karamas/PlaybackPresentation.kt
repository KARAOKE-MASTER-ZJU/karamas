package zju.idol.karamas

import android.annotation.SuppressLint
import android.app.Activity
import android.app.Presentation
import android.graphics.Color
import android.hardware.display.DisplayManager
import android.net.Uri
import android.view.Display
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.webkit.*
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.webkit.WebMessageCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import app.tauri.annotation.InvokeArg
import org.json.JSONObject
import java.util.UUID

@InvokeArg
class PlaybackOutputArgs {
    var output: String = "embedded"
    var token: String = ""
    var message: Map<String, Any?> = emptyMap()
}

// 将承载播放器的 RustWebView 迁到外接屏，Activity 中显示控制页面。
class PlaybackPresentation(private val activity: Activity) {
    private var host: RustWebView? = null
    private var parent: ViewGroup? = null
    private var parentIndex = 0
    private var hostLayout: ViewGroup.LayoutParams? = null
    private var presentation: Presentation? = null
    private var controller: WebView? = null
    private var token: String? = null

    fun attach(webView: WebView) { close(); host = webView as RustWebView }

    @Suppress("DEPRECATION")
    fun displays(): List<Display> = activity.getSystemService(DisplayManager::class.java)
        .getDisplays(DisplayManager.DISPLAY_CATEGORY_PRESENTATION)
        .filter { it.isValid && it.displayId != activity.windowManager.defaultDisplay.displayId }

    // 先加载隐藏的控制页面，Rust 收到前端就绪通知后再调用 attachOutput。
    @SuppressLint("SetJavaScriptEnabled")
    fun prepare(output: String) {
        check(output.startsWith("screen:")) { "Android 请选择外接屏幕。" }
        check(displays().any { "screen:${it.displayId}" == output }) { "所选屏幕已变化，请刷新屏幕列表。" }
        if (controller != null) return
        check(WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) {
            "请更新 Android System WebView 后使用外屏播放。"
        }
        val host = checkNotNull(host) { "主页面尚未加载。" }
        val parent = host.parent as ViewGroup
        // 保存原视图位置和布局，返回内嵌时放回 Activity 中的同一位置。
        this.parent = parent
        parentIndex = parent.indexOfChild(host)
        hostLayout = host.layoutParams
        val page = Uri.parse(checkNotNull(host.url)).buildUpon().path("/").clearQuery()
            .appendQueryParameter("playback-controller", "").fragment(null).build()
        val origin = "${page.scheme}://${page.encodedAuthority}"
        val session = UUID.randomUUID().toString()
        token = session
        val view = WebView(activity)
        controller = view
        view.visibility = View.INVISIBLE
        view.setBackgroundColor(Color.BLACK)
        view.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            allowFileAccess = false
            allowContentAccess = false
        }
        // 控制页面经原 RustWebView 调用 Tauri，回复按本次页面的 token 匹配。
        WebViewCompat.addWebMessageListener(view, "KaramasController", setOf(origin)) {
            _, message, source, mainFrame, _ ->
            if (!mainFrame || source.toString() != origin || controller !== view ||
                message.type != WebMessageCompat.TYPE_STRING) return@addWebMessageListener
            val data = try { JSONObject(message.data ?: return@addWebMessageListener) }
                catch (_: Exception) { return@addWebMessageListener }
            data.put("token", session)
            host.evaluateJavascript("window.dispatchEvent(new CustomEvent('android-controller-call',{detail:$data}))", null)
        }
        // 控制页面复用 Tauri 的资源请求处理，加载同源的前端资源。
        val assets = RustWebViewClient(host, activity)
        view.webViewClient = object : WebViewClient() {
            override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? =
                assets.shouldInterceptRequest(host, request)
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean =
                request.url.scheme != page.scheme || request.url.encodedAuthority != page.encodedAuthority
            override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
                if (request.isForMainFrame && controller === view) close()
            }
            override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
                if (controller === view) close()
                return true
            }
        }
        parent.addView(view, hostLayout)
        view.loadUrl(page.toString())
    }

    fun attachOutput(output: String) {
        if (output == "embedded") { close(); return }
        val host = checkNotNull(host)
        val display = displays().firstOrNull { "screen:${it.displayId}" == output }
            ?: error("所选屏幕已变化，请刷新屏幕列表。")
        val old = presentation
        // 归位回调随当前 Presentation 绑定，切换屏幕时先解除旧绑定。
        old?.setOnDismissListener(null)
        (host.parent as? ViewGroup)?.removeView(host)
        val next = Presentation(activity, display, android.R.style.Theme_Black_NoTitleBar_Fullscreen)
        presentation = next
        next.setContentView(host, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        next.window?.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON or
            WindowManager.LayoutParams.FLAG_HARDWARE_ACCELERATED or WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE)
        next.setOnDismissListener { if (presentation === next) close() }
        try {
            next.show()
            next.window?.let { WindowCompat.getInsetsController(it, it.decorView).hide(WindowInsetsCompat.Type.systemBars()) }
            controller?.visibility = View.VISIBLE
            old?.dismiss()
        } catch (error: Exception) { close(); throw error }
    }

    fun send(expectedToken: String, message: Map<String, Any?>) {
        if (token != expectedToken) return
        controller?.evaluateJavascript("window.dispatchEvent(new CustomEvent('android-controller-message',{detail:${JSONObject(message)}}))", null)
    }

    fun close(): Boolean {
        if (controller == null && presentation == null) return false
        val previous = presentation
        previous?.setOnDismissListener(null)
        presentation = null
        val host = host
        // 先将播放视图放回 Activity，再销毁控制页面和外屏容器。
        if (host != null && parent != null && host.parent !== parent) {
            (host.parent as? ViewGroup)?.removeView(host)
            parent?.addView(host, parentIndex, hostLayout)
        }
        val view = controller
        controller = null
        token = null
        (view?.parent as? ViewGroup)?.removeView(view)
        view?.stopLoading()
        view?.destroy()
        previous?.dismiss()
        parent = null
        hostLayout = null
        // 通知播放文档清理桥接资源，并回报已返回内嵌。
        host?.evaluateJavascript("window.dispatchEvent(new Event('android-controller-reset'));window.dispatchEvent(new Event('android-playback-returned'))", null)
        return true
    }

    fun setForeground(foreground: Boolean) {
        // 外屏输出期间保留媒体文档的运行状态。
        if (presentation != null || foreground) host?.onResume()
    }
    fun destroy() { close(); host = null }
}
