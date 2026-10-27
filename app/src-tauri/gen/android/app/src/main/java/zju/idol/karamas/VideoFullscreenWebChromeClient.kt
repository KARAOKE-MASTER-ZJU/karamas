package zju.idol.karamas

import android.content.pm.ActivityInfo
import android.graphics.Bitmap
import android.net.Uri
import android.view.View
import android.view.ViewGroup
import android.webkit.ConsoleMessage
import android.webkit.GeolocationPermissions
import android.webkit.JsPromptResult
import android.webkit.JsResult
import android.webkit.PermissionRequest
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebView
import android.widget.FrameLayout
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

// 接管 WebView 的原生全屏视图；浏览器权限、对话框和文件选择仍由 Wry 处理。
class VideoFullscreenWebChromeClient(
    private val activity: AppCompatActivity,
    private val delegate: WebChromeClient,
) : WebChromeClient() {
    private data class FullscreenState(
        val view: View,
        val callback: CustomViewCallback,
        val orientation: Int,
        val barsBehavior: Int,
        val statusBarVisible: Boolean,
        val navigationBarVisible: Boolean,
    )

    private var fullscreen: FullscreenState? = null
    private val backCallback = object : OnBackPressedCallback(true) {
        override fun handleOnBackPressed() = onHideCustomView()
    }

    override fun onShowCustomView(view: View, callback: CustomViewCallback) {
        if (fullscreen != null) {
            callback.onCustomViewHidden()
            return
        }
        val decor = activity.window.decorView as ViewGroup
        val controller = WindowCompat.getInsetsController(activity.window, decor)
        val insets = ViewCompat.getRootWindowInsets(decor)
        fullscreen = FullscreenState(
            view,
            callback,
            activity.requestedOrientation,
            controller.systemBarsBehavior,
            insets?.isVisible(WindowInsetsCompat.Type.statusBars()) ?: true,
            insets?.isVisible(WindowInsetsCompat.Type.navigationBars()) ?: true,
        )
        decor.addView(view, FrameLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.MATCH_PARENT,
        ))
        // 进入全屏时最后注册，优先于 Tauri 的页面返回和退到桌面处理。
        activity.onBackPressedDispatcher.addCallback(activity, backCallback)
        activity.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        controller.systemBarsBehavior =
            WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        controller.hide(WindowInsetsCompat.Type.systemBars())
        view.requestFocus()
    }

    override fun onHideCustomView() {
        val previous = fullscreen ?: return
        fullscreen = null
        backCallback.remove()
        (previous.view.parent as? ViewGroup)?.removeView(previous.view)
        activity.requestedOrientation = previous.orientation
        val decor = activity.window.decorView
        val controller = WindowCompat.getInsetsController(activity.window, decor)
        controller.systemBarsBehavior = previous.barsBehavior
        if (previous.statusBarVisible) controller.show(WindowInsetsCompat.Type.statusBars())
        else controller.hide(WindowInsetsCompat.Type.statusBars())
        if (previous.navigationBarVisible) controller.show(WindowInsetsCompat.Type.navigationBars())
        else controller.hide(WindowInsetsCompat.Type.navigationBars())
        previous.callback.onCustomViewHidden()
        ViewCompat.requestApplyInsets(decor)
    }

    override fun onPermissionRequest(request: PermissionRequest) =
        delegate.onPermissionRequest(request)

    override fun getDefaultVideoPoster(): Bitmap? = delegate.defaultVideoPoster

    override fun onJsAlert(view: WebView, url: String, message: String, result: JsResult): Boolean =
        delegate.onJsAlert(view, url, message, result)

    override fun onJsConfirm(view: WebView, url: String, message: String, result: JsResult): Boolean =
        delegate.onJsConfirm(view, url, message, result)

    override fun onJsPrompt(
        view: WebView, url: String, message: String, defaultValue: String, result: JsPromptResult,
    ): Boolean = delegate.onJsPrompt(view, url, message, defaultValue, result)

    override fun onGeolocationPermissionsShowPrompt(
        origin: String, callback: GeolocationPermissions.Callback,
    ) = delegate.onGeolocationPermissionsShowPrompt(origin, callback)

    override fun onShowFileChooser(
        webView: WebView,
        filePathCallback: ValueCallback<Array<Uri?>?>,
        fileChooserParams: FileChooserParams,
    ): Boolean = delegate.onShowFileChooser(webView, filePathCallback, fileChooserParams)

    override fun onConsoleMessage(consoleMessage: ConsoleMessage): Boolean =
        delegate.onConsoleMessage(consoleMessage)

    override fun onReceivedTitle(view: WebView, title: String) =
        delegate.onReceivedTitle(view, title)
}
