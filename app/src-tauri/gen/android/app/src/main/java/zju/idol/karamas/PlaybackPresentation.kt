package zju.idol.karamas

import android.annotation.SuppressLint
import android.app.Activity
import android.app.Presentation
import android.graphics.Color
import android.hardware.display.DisplayManager
import android.net.Uri
import android.view.Display
import android.view.WindowManager
import android.webkit.*
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.webkit.WebMessageCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import app.tauri.annotation.InvokeArg
import app.tauri.plugin.Channel
import app.tauri.plugin.JSObject
import org.json.JSONObject

@InvokeArg
class PlaybackWindowArgs {
    var token: String = ""
    var displayId: String? = null
    var onMessage: Channel? = null
    var message: Map<String, Any?> = emptyMap()
}

class PlaybackPresentation(private val activity: Activity) {
    private var host: RustWebView? = null
    private var dialog: Presentation? = null
    private var output: WebView? = null
    private var token: String? = null
    private var onMessage: Channel? = null

    fun attach(webView: WebView) {
        close()
        host = webView as RustWebView
    }

    @Suppress("DEPRECATION")
    fun displays(): List<Display> = activity.getSystemService(DisplayManager::class.java)
        .getDisplays(DisplayManager.DISPLAY_CATEGORY_PRESENTATION)
        .filter { it.isValid && it.displayId != activity.windowManager.defaultDisplay.displayId }

    @SuppressLint("SetJavaScriptEnabled")
    fun open(args: PlaybackWindowArgs) {
        check(dialog == null) { "播放窗口已打开。" }
        check(WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER) &&
            WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
            "请更新 Android System WebView 后使用外屏播放。"
        }
        val host = checkNotNull(host) { "主页面尚未加载。" }
        val display = displays().firstOrNull { "screen:${it.displayId}" == args.displayId }
            ?: error("所选屏幕已变化，请刷新屏幕列表。")
        val page = Uri.parse(checkNotNull(host.url)).buildUpon().path("/").clearQuery()
            .fragment(null).build()
        val origin = "${page.scheme}://${page.encodedAuthority}"
        val presentation = Presentation(activity, display, android.R.style.Theme_Black_NoTitleBar_Fullscreen)
        val view = WebView(presentation.context)
        token = args.token
        onMessage = checkNotNull(args.onMessage)
        dialog = presentation
        output = view
        try {
            view.setBackgroundColor(Color.BLACK)
            view.settings.apply {
                javaScriptEnabled = true
                domStorageEnabled = true
                mediaPlaybackRequiresUserGesture = false
                allowFileAccess = false
                allowContentAccess = false
            }
            WebViewCompat.addDocumentStartJavaScript(view,
                "window.__KARAMAS_PLAYBACK_WINDOW__ = ${JSONObject.quote(args.token)};", setOf(origin))
            WebViewCompat.addWebMessageListener(view, "KaramasPlayback", setOf(origin)) {
                _, message, source, mainFrame, _ ->
                if (!mainFrame || source.toString() != origin || output !== view ||
                    Uri.parse(view.url ?: "").buildUpon().fragment(null).build() != page || message.type != WebMessageCompat.TYPE_STRING) return@addWebMessageListener
                val data = try { JSObject(message.data ?: return@addWebMessageListener) }
                    catch (_: Exception) { return@addWebMessageListener }
                onMessage?.send(data.put("token", args.token))
            }
            // 复用 Tauri 的资源请求处理，加载打包资源和媒体代理。
            val assets = RustWebViewClient(host, activity)
            view.webViewClient = object : WebViewClient() {
                override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? =
                    assets.shouldInterceptRequest(host, request)

                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean =
                    request.url != page

                override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
                    if (request.isForMainFrame && output === view) close()
                }

                override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
                    if (output === view) close()
                    return true
                }
            }
            presentation.setContentView(view)
            presentation.window?.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON or
                WindowManager.LayoutParams.FLAG_HARDWARE_ACCELERATED or WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE)
            presentation.setOnDismissListener { if (dialog === presentation) close() }
            presentation.show()
            presentation.window?.let {
                WindowCompat.getInsetsController(it, it.decorView).hide(WindowInsetsCompat.Type.systemBars())
            }
            view.loadUrl(page.toString())
        } catch (error: Exception) {
            close()
            throw error
        }
    }

    fun send(message: Map<String, Any?>) {
        check(token != null && message["token"] == token) { "播放窗口已关闭。" }
        output?.evaluateJavascript(
            "window.dispatchEvent(new CustomEvent('playback-window-message',{detail:${JSONObject(message)}}))", null)
    }

    fun focus(expectedToken: String) {
        check(token == expectedToken && dialog != null) { "播放窗口已关闭。" }
        dialog?.show()
    }

    fun close(expectedToken: String? = null): Boolean {
        if (token == null || (expectedToken != null && token != expectedToken)) return false
        val previous = dialog
        val view = output
        val message = JSObject().put("type", "message").put("token", token).put("message",
            JSObject().put("token", token).put("type", "closed"))
        token = null
        dialog = null
        output = null
        previous?.setOnDismissListener(null)
        previous?.dismiss()
        view?.stopLoading()
        view?.destroy()
        onMessage?.send(message)
        onMessage = null
        return true
    }

    fun setForeground(foreground: Boolean) {
        if (foreground) output?.onResume() else output?.onPause()
    }

    fun destroy() {
        close()
        host = null
    }
}
