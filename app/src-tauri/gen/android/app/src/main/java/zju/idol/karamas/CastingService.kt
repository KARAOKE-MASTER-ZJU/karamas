package zju.idol.karamas

import android.annotation.SuppressLint
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Bundle
import android.os.IBinder
import android.os.PowerManager
import android.os.ResultReceiver
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

// 播放调度仍由 Rust 会话负责，服务只管理 Android 通知和锁屏期间的 CPU 唤醒。
class CastingService : Service() {
    private var wakeLock: PowerManager.WakeLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            getSystemService(NotificationManager::class.java).createNotificationChannel(
                NotificationChannel(CHANNEL, getString(R.string.casting_channel_name), NotificationManager.IMPORTANCE_LOW)
            )
        }
    }

    @SuppressLint("WakelockTimeout")
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent == null) {
            stopSelf(startId)
            return START_NOT_STICKY
        }
        val result = if (Build.VERSION.SDK_INT >= 33) {
            intent.getParcelableExtra(EXTRA_RESULT, ResultReceiver::class.java)
        } else {
            @Suppress("DEPRECATION")
            intent.getParcelableExtra<ResultReceiver>(EXTRA_RESULT)
        }
        try {
            val openApp = PendingIntent.getActivity(
                this, 0,
                Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
            )
            val notification = NotificationCompat.Builder(this, CHANNEL)
                .setSmallIcon(android.R.drawable.ic_media_play)
                .setContentTitle(getString(R.string.app_name))
                .setContentText(getString(R.string.casting_service_content, intent.getStringExtra(EXTRA_ROOM) ?: ""))
                .setContentIntent(openApp)
                .setCategory(NotificationCompat.CATEGORY_TRANSPORT)
                .setPriority(NotificationCompat.PRIORITY_LOW)
                .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
                .setOnlyAlertOnce(true)
                .setOngoing(true)
                .setSilent(true)
                .build()
            val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
            } else 0
            ServiceCompat.startForeground(this, 1, notification, type)
            if (wakeLock?.isHeld != true) {
                wakeLock = getSystemService(PowerManager::class.java)
                    .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "karamas:casting")
                    .apply {
                        setReferenceCounted(false)
                        // 只在投屏会话存续期间持有，停止服务时统一释放。
                        acquire()
                    }
            }
            Log.i("CASTING_SERVICE", "投屏前台服务已启动")
            result?.send(0, Bundle())
        } catch (error: Exception) {
            Log.e("CASTING_SERVICE", "无法启动投屏前台服务", error)
            result?.send(1, Bundle().apply { putString("error", error.message) })
            stopSelf(startId)
        }
        // 不保存旧房间，也不在进程被终止后自动重启空会话。
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
        stopForeground(STOP_FOREGROUND_REMOVE)
        Log.i("CASTING_SERVICE", "投屏前台服务已停止")
        super.onDestroy()
    }

    companion object {
        private const val CHANNEL = "casting"
        const val EXTRA_ROOM = "roomId"
        const val EXTRA_RESULT = "result"
    }
}
