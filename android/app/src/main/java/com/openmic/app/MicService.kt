package com.openmic.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import kotlin.concurrent.thread
import kotlin.math.sqrt

class MicService : Service() {

    companion object {
        const val ACTION_LEVEL = "com.openmic.app.LEVEL"
        const val EXTRA_RMS = "rms"
        const val EXTRA_STATE = "state"
        const val SAMPLE_RATE = 48000
        const val FRAME_SAMPLES = 960 // 20 мс
        private const val CH = "openmic"
        private const val NOTIF_ID = 1

        /** Глобально доступное последнее состояние сервиса для мгновенного считывания UI */
        @Volatile var currentState: String = "остановлен"
            private set
        @Volatile var isRunning: Boolean = false
            private set
    }

    private var worker: Thread? = null
    @Volatile private var running = false
    // Поколение воркера: стоп+старт быстрее join() не должны давать два эфира.
    @Volatile private var gen = 0
    private var wakeLock: PowerManager.WakeLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CH, "OpenMic", NotificationManager.IMPORTANCE_LOW)
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == "STOP") {
            stopStream()
            stopSelf()
            return START_NOT_STICKY
        }
        val host = intent?.getStringExtra("host").orEmpty()
        val port = intent?.getIntExtra("port", 59100) ?: 59100
        val token = intent?.getStringExtra("token").orEmpty()
        if (host.isBlank() || token.isBlank()) return START_STICKY
        val monitor = intent?.getBooleanExtra("monitor", false) ?: false
        val monitorVol = intent?.getIntExtra("monitorVol", 70) ?: 70
        val ns = intent?.getBooleanExtra("ns", false) ?: false
        gen++ // старый воркер (если пережил join) увидит чужое поколение и выйдет
        startStream(host, port, token, monitor, monitorVol, ns, gen)
        return START_STICKY
    }

    private fun notif(text: String): Notification =
        Notification.Builder(this, CH)
            .setContentTitle("OpenMic")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setOngoing(true)
            .build()

    private fun startStream(
        host: String, port: Int, token: String,
        monitor: Boolean, monitorVol: Int, ns: Boolean,
        myGen: Int,
    ) {
        fun alive() = running && gen == myGen
        running = true
        isRunning = true
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(
                NOTIF_ID, notif("Эфир: $host:$port"),
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE,
            )
        } else {
            startForeground(NOTIF_ID, notif("Эфир: $host:$port"))
        }
        val pm = getSystemService(PowerManager::class.java)
        wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "OpenMic:mic").apply {
            acquire(3600_000L)
        }
        broadcast(0f, "подключение…")
        worker = thread(name = "openmic-send") {
            android.util.Log.i("OpenMic", "worker start gen=$myGen")
            var rec: AudioRecord? = null
            var track: android.media.AudioTrack? = null
            var suppressor: android.media.audiofx.NoiseSuppressor? = null
            val nsOnAir = java.util.concurrent.atomic.AtomicBoolean(ns)
            val client = NetClient(host, port, token)
            client.onControl = { type, value ->
                if (type == "ns") {
                    nsOnAir.set(value)
                    try {
                        suppressor?.enabled = value
                    } catch (_: Exception) {}
                    android.util.Log.i("OpenMic", "ns set from desktop: $value")
                    broadcast(0f, if (value) "эфир (шумодав вкл)" else "эфир")
                }
            }
            try {
                // Соединение открывает только цикл ниже (иначе два сокета).
                val minBuf = AudioRecord.getMinBufferSize(
                    SAMPLE_RATE,
                    AudioFormat.CHANNEL_IN_MONO,
                    AudioFormat.ENCODING_PCM_16BIT,
                )
                // Используем стандартный физический микрофон MIC (как в scrcpy)
                rec = AudioRecord(
                    MediaRecorder.AudioSource.MIC,
                    SAMPLE_RATE,
                    AudioFormat.CHANNEL_IN_MONO,
                    AudioFormat.ENCODING_PCM_16BIT,
                    maxOf(minBuf * 2, FRAME_SAMPLES * 2 * 4),
                )
                if (rec.state != AudioRecord.STATE_INITIALIZED) {
                    throw IllegalStateException("AudioRecord init failed")
                }
                // Создаём всегда (если есть), чтобы десктоп мог включить на лету.
                try {
                    if (android.media.audiofx.NoiseSuppressor.isAvailable()) {
                        suppressor = android.media.audiofx.NoiseSuppressor.create(
                            rec.audioSessionId
                        )
                        suppressor?.enabled = nsOnAir.get()
                        android.util.Log.i("OpenMic", "noise suppressor ready, on=${nsOnAir.get()}")
                    } else {
                        android.util.Log.i("OpenMic", "noise suppressor unavailable")
                    }
                } catch (e: Exception) {
                    android.util.Log.i("OpenMic", "ns failed: ${e.message}")
                }
                if (monitor) {
                    // Самоконтроль: дубль голоса в наушники, low-latency.
                    val attrs = android.media.AudioAttributes.Builder()
                        .setUsage(android.media.AudioAttributes.USAGE_MEDIA)
                        .setContentType(android.media.AudioAttributes.CONTENT_TYPE_SPEECH)
                        .build()
                    val fmt = AudioFormat.Builder()
                        .setSampleRate(SAMPLE_RATE)
                        .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .build()
                    val minTrack = android.media.AudioTrack.getMinBufferSize(
                        SAMPLE_RATE,
                        AudioFormat.CHANNEL_OUT_MONO,
                        AudioFormat.ENCODING_PCM_16BIT,
                    )
                    track = android.media.AudioTrack.Builder()
                        .setAudioAttributes(attrs)
                        .setAudioFormat(fmt)
                        .setBufferSizeInBytes(maxOf(minTrack * 2, FRAME_SAMPLES * 2 * 4))
                        .setTransferMode(android.media.AudioTrack.MODE_STREAM)
                        .setPerformanceMode(android.media.AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
                        .build()
                    track.setVolume(monitorVol / 100f)
                    track.play()
                    android.util.Log.i("OpenMic", "monitor on, vol=$monitorVol")
                }
                rec.startRecording()
                val pcm = ByteArray(FRAME_SAMPLES * 2)
                var backoff = 1000L
                // Микрофон и самоконтроль живут всё время; TCP переподключается
                // с backoff 1→2→5→10 с. Демон в паузе отдаёт тишину.
                while (alive()) {
                    try {
                        client.connect()
                    } catch (e: Exception) {
                        broadcast(0f, "переподключение через ${backoff / 1000} с…")
                        sleepQuietly(backoff)
                        backoff = minOf(backoff * 2, 10000L)
                        continue
                    }
                    backoff = 1000L
                    broadcast(0f, "эфир")
                    var off = 0
                    var linked = true
                    while (alive() && linked) {
                        val n = rec.read(pcm, off, pcm.size - off)
                        if (n <= 0) continue
                        off += n
                        if (off < pcm.size) continue
                        off = 0
                        track?.write(pcm, 0, pcm.size) // best-effort дубль в наушники
                        try {
                            client.sendFrame(pcm, pcm.size)
                        } catch (_: Exception) {
                            linked = false // обрыв — наружу, в backoff-цикл
                        }
                        if (linked) broadcast(rms(pcm), "эфир")
                    }
                    client.close()
                    if (alive()) broadcast(0f, "переподключение…")
                }
            } catch (e: Exception) {
                broadcast(0f, "ошибка: ${e.message}")
            } finally {
                try { rec?.stop() } catch (_: Exception) {}
                try { rec?.release() } catch (_: Exception) {}
                try { suppressor?.release() } catch (_: Exception) {}
                try { track?.stop() } catch (_: Exception) {}
                try { track?.release() } catch (_: Exception) {}
                client.close()
                if (gen == myGen) {
                    // Только актуальное поколение гасит сервис; протухшее молча выходит.
                    running = false
                    isRunning = false
                    wakeLock?.release()
                    stopForeground(STOP_FOREGROUND_REMOVE)
                }
                android.util.Log.i("OpenMic", "worker exit gen=$myGen")
            }
        }
    }

    private fun stopStream() {
        gen++ // протухаем любой зависший воркер
        running = false
        isRunning = false
        worker?.join(1500)
        worker = null
        broadcast(0f, "остановлен")
    }

    private fun sleepQuietly(ms: Long) {
        try {
            Thread.sleep(ms)
        } catch (_: InterruptedException) {}
    }

    private fun broadcast(rms: Float, state: String) {
        currentState = state
        val intent = Intent(ACTION_LEVEL)
            .setPackage(packageName)
            .putExtra(EXTRA_RMS, rms)
            .putExtra(EXTRA_STATE, state)
        sendBroadcast(intent)
    }

    private fun rms(pcm: ByteArray): Float {
        var sum = 0.0
        var n = 0
        var i = 0
        while (i + 1 < pcm.size) {
            val s = ((pcm[i + 1].toInt() shl 8) or (pcm[i].toInt() and 0xFF)).toShort().toDouble() / 32768.0
            sum += s * s
            n++
            i += 2
        }
        return if (n == 0) 0f else sqrt(sum / n).toFloat()
    }
}
