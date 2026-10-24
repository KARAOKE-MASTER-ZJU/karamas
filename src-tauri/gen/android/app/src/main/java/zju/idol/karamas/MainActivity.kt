package zju.idol.karamas

import android.content.Intent
import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onDestroy() {
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
