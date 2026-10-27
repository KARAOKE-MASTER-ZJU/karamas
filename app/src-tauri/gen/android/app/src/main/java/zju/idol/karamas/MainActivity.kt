package zju.idol.karamas

import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  private var videoFullscreen: VideoFullscreenWebChromeClient? = null

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    // Wry 在此回调之后设置默认客户端，等初始化完成再接入全屏处理。
    webView.post {
      if (isDestroyed) return@post
      val delegate = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
        webView.webChromeClient
      } else {
        null
      } ?: RustWebChromeClient(this, (webView as RustWebView).id)
      videoFullscreen = VideoFullscreenWebChromeClient(this, delegate)
      webView.webChromeClient = videoFullscreen
    }
  }

  override fun onDestroy() {
    videoFullscreen?.onHideCustomView()
    // 配置变化保留服务，页面最终销毁时与 Rust 的退出清理一起停止服务。
    if (!isChangingConfigurations) stopService(Intent(this, CastingService::class.java))
    super.onDestroy()
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // 系统栏和刘海占用的区域留给原生容器，避免遮住网页标题和底部导航。
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val types = WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      val bars = insets.getInsets(types)
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.Builder(insets).setInsets(types, Insets.NONE).build()
    }
    ViewCompat.requestApplyInsets(content)
  }
}
