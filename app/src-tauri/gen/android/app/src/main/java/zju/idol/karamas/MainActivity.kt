package zju.idol.karamas

import android.content.Context
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
  private external fun initializePlatformVerifier(context: Context)

  internal val playbackPresentation by lazy { PlaybackPresentation(this) }
  private var videoFullscreen: VideoFullscreenWebChromeClient? = null

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    playbackPresentation.attach(webView)
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

  override fun onPause() {
    playbackPresentation.setForeground(false)
    super.onPause()
  }

  override fun onResume() {
    super.onResume()
    playbackPresentation.setForeground(true)
  }

  override fun onDestroy() {
    playbackPresentation.destroy()
    videoFullscreen?.onHideCustomView()
    // 配置变化保留服务，页面最终销毁时与 Rust 的退出清理一起停止服务。
    if (!isChangingConfigurations) stopService(Intent(this, CastingService::class.java))
    super.onDestroy()
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    // 父类 onCreate 会启动 Rust，须先加载原生库并初始化证书验证器。
    System.loadLibrary("karamas_lib")
    initializePlatformVerifier(applicationContext)
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
