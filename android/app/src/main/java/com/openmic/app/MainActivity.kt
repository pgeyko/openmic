package com.openmic.app

import android.Manifest
import android.animation.ObjectAnimator
import android.animation.ValueAnimator
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.os.Bundle
import android.transition.TransitionManager
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.*
import com.google.zxing.integration.android.IntentIntegrator
import org.json.JSONObject

class MainActivity : android.app.Activity() {

    private lateinit var rootScroll: ViewGroup
    private lateinit var ip: EditText
    private lateinit var port: EditText
    private lateinit var token: EditText
    private lateinit var level: WaveView
    private lateinit var state: TextView
    private lateinit var onAirTarget: TextView
    private lateinit var dot: View
    private lateinit var spinner: ProgressBar
    private lateinit var connectionSection: View
    private lateinit var manualFields: View
    private lateinit var toggleManual: TextView
    private lateinit var startBtn: Button
    private lateinit var stopBtn: Button
    private lateinit var volLabel: TextView
    private lateinit var monitorVol: SeekBar

    private var pulse: ObjectAnimator? = null

    private val rx = object : BroadcastReceiver() {
        override fun onReceive(c: Context?, i: Intent?) {
            if (i?.action != MicService.ACTION_LEVEL) return
            val r = i.getFloatExtra(MicService.EXTRA_RMS, 0f)
            level.push(r)
            val st = i.getStringExtra(MicService.EXTRA_STATE) ?: ""
            setStatus(st)
        }
    }

    private fun dotColor(argb: Int) {
        val bg = dot.background as? GradientDrawable
        if (bg != null) {
            bg.setColor(argb)
        } else {
            dot.background = GradientDrawable().apply {
                shape = GradientDrawable.OVAL
                setColor(argb)
            }
        }
    }

    private fun setStatus(text: String) {
        state.text = text
        pulse?.cancel()
        pulse = null
        dot.alpha = 1f
        spinner.visibility = View.GONE

        val isAir = text.startsWith("эфир")
        val isConnecting = text.contains("переподключ") || text.contains("подключение")
        val isError = text.startsWith("ошибка")

        when {
            isAir -> {
                dotColor(getColor(R.color.accent_green))
                updateUIMode(onAir = true)
            }
            isConnecting -> {
                dotColor(getColor(R.color.accent_amber))
                spinner.visibility = View.VISIBLE
                pulse = ObjectAnimator.ofFloat(dot, "alpha", 1f, 0.2f).apply {
                    duration = 500
                    repeatMode = ValueAnimator.REVERSE
                    repeatCount = ValueAnimator.INFINITE
                    start()
                }
                updateUIMode(onAir = true)
            }
            isError -> {
                dotColor(getColor(R.color.accent_red))
                updateUIMode(onAir = false)
            }
            else -> {
                dotColor(0xFF8E8E93.toInt())
                level.clear()
                updateUIMode(onAir = false)
            }
        }
    }

    /** Плавное переключение UI: режим настройки vs режим активного эфира */
    private fun updateUIMode(onAir: Boolean) {
        if (onAir) {
            if (startBtn.visibility != View.GONE) {
                TransitionManager.beginDelayedTransition(rootScroll)
                startBtn.visibility = View.GONE
                stopBtn.visibility = View.VISIBLE
                connectionSection.visibility = View.GONE
                onAirTarget.visibility = View.VISIBLE
                onAirTarget.text = "ПК: ${ip.text}:${port.text}"
            }
        } else {
            if (stopBtn.visibility != View.GONE) {
                TransitionManager.beginDelayedTransition(rootScroll)
                stopBtn.visibility = View.GONE
                startBtn.visibility = View.VISIBLE
                connectionSection.visibility = View.VISIBLE
                onAirTarget.visibility = View.GONE
            }
        }
    }

    override fun onCreate(b: Bundle?) {
        super.onCreate(b)
        setContentView(R.layout.activity_main)

        rootScroll = findViewById(R.id.rootScroll)
        ip = findViewById(R.id.ip)
        port = findViewById(R.id.port)
        token = findViewById(R.id.token)
        level = findViewById(R.id.level)
        state = findViewById(R.id.state)
        onAirTarget = findViewById(R.id.onAirTarget)
        dot = findViewById(R.id.dot)
        spinner = findViewById(R.id.spinner)
        connectionSection = findViewById(R.id.connectionSection)
        manualFields = findViewById(R.id.manualFields)
        toggleManual = findViewById(R.id.toggleManual)
        startBtn = findViewById(R.id.start)
        stopBtn = findViewById(R.id.stop)
        volLabel = findViewById(R.id.volLabel)
        monitorVol = findViewById(R.id.monitorVol)

        val prefs = getPreferences(Context.MODE_PRIVATE)
        val savedIp = prefs.getString("ip", "").orEmpty()
        val savedPort = prefs.getString("port", "59100").orEmpty()
        val savedToken = prefs.getString("token", "").orEmpty()

        ip.setText(savedIp)
        port.setText(savedPort)
        token.setText(savedToken)

        val rbWifi = findViewById<RadioButton>(R.id.rbWifi)
        val rbUsb = findViewById<RadioButton>(R.id.rbUsb)
        val isUsbMode = prefs.getBoolean("isUsb", false)
        if (isUsbMode) rbUsb.isChecked = true else rbWifi.isChecked = true

        rbUsb.setOnCheckedChangeListener { _, isChecked ->
            prefs.edit().putBoolean("isUsb", isChecked).apply()
            if (isChecked) {
                // В USB-режиме хост всегда локальный (через adb forward)
                ip.setText("127.0.0.1")
            } else {
                ip.setText(prefs.getString("savedWifiIp", savedIp))
            }
        }
        rbWifi.setOnCheckedChangeListener { _, isChecked ->
            if (isChecked) {
                prefs.edit().putBoolean("isUsb", false).apply()
                ip.setText(prefs.getString("savedWifiIp", savedIp))
            }
        }

        // Если токен уже есть — поля ручного ввода скрыты, показываем сканер. Если пустой — открываем.
        if (savedToken.isBlank()) {
            manualFields.visibility = View.VISIBLE
            toggleManual.text = "Скрыть ручной ввод"
        }

        toggleManual.setOnClickListener {
            TransitionManager.beginDelayedTransition(rootScroll)
            if (manualFields.visibility == View.VISIBLE) {
                manualFields.visibility = View.GONE
                toggleManual.text = "Ввести параметры вручную"
            } else {
                manualFields.visibility = View.VISIBLE
                toggleManual.text = "Скрыть ручной ввод"
            }
        }

        val monitor = findViewById<CheckBox>(R.id.monitor)
        val monitorSettings = findViewById<View>(R.id.monitorSettings)
        val isMonitorEnabled = prefs.getBoolean("monitor", false)
        monitor.isChecked = isMonitorEnabled
        monitorSettings.visibility = if (isMonitorEnabled) View.VISIBLE else View.GONE
        monitor.setOnCheckedChangeListener { _, isChecked ->
            TransitionManager.beginDelayedTransition(rootScroll)
            monitorSettings.visibility = if (isChecked) View.VISIBLE else View.GONE
        }
        val savedVol = prefs.getInt("monitorVol", 70)
        monitorVol.progress = savedVol
        volLabel.text = "Громкость наушников: $savedVol%"
        monitorVol.setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
            override fun onProgressChanged(sb: SeekBar?, v: Int, fromUser: Boolean) {
                volLabel.text = "Громкость наушников: $v%"
            }
            override fun onStartTrackingTouch(sb: SeekBar?) {}
            override fun onStopTrackingTouch(sb: SeekBar?) {}
        })

        val sources = listOf("VOICE_RECOGNITION", "UNPROCESSED", "CAMCORDER", "VOICE_COMMUNICATION")
        val source = findViewById<Spinner>(R.id.source)
        source.adapter = ArrayAdapter(this, android.R.layout.simple_spinner_item, sources).apply {
            setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
        }
        source.setSelection(prefs.getInt("sourceIdx", 0))

        val ns = findViewById<CheckBox>(R.id.ns)
        ns.isChecked = prefs.getBoolean("ns", false)
        if (!android.media.audiofx.NoiseSuppressor.isAvailable()) {
            ns.isChecked = false
            ns.isEnabled = false
            ns.text = "Шумодав (не поддерживается устройством)"
        }

        val keepScreen = findViewById<CheckBox>(R.id.keepScreen)
        keepScreen.isChecked = prefs.getBoolean("keepScreen", false)
        applyKeepScreen(keepScreen.isChecked)
        keepScreen.setOnCheckedChangeListener { _, on ->
            prefs.edit().putBoolean("keepScreen", on).apply()
            applyKeepScreen(on)
        }

        requestPerms()

        startBtn.setOnClickListener {
            val currentIp = ip.text.toString().trim()
            val edit = prefs.edit()
                .putString("ip", currentIp)
                .putString("port", port.text.toString().trim())
                .putString("token", token.text.toString().trim())
                .putBoolean("monitor", monitor.isChecked)
                .putInt("monitorVol", monitorVol.progress)
                .putInt("sourceIdx", source.selectedItemPosition)
                .putBoolean("ns", ns.isChecked)
            if (currentIp != "127.0.0.1") {
                edit.putString("savedWifiIp", currentIp)
            }
            edit.apply()

            val svc = Intent(this, MicService::class.java)
                .putExtra("host", ip.text.toString().trim())
                .putExtra("port", port.text.toString().trim().toIntOrNull() ?: 59100)
                .putExtra("token", token.text.toString().trim())
                .putExtra("monitor", monitor.isChecked)
                .putExtra("monitorVol", monitorVol.progress)
                .putExtra("audioSource", audioSourceAt(source.selectedItemPosition))
                .putExtra("ns", ns.isChecked)

            setStatus("подключение…")
            if (Build.VERSION.SDK_INT >= 26) startForegroundService(svc) else startService(svc)
        }

        stopBtn.setOnClickListener {
            setStatus("остановлен")
            startService(Intent(this, MicService::class.java).setAction("STOP"))
        }

        findViewById<Button>(R.id.scan).setOnClickListener {
            IntentIntegrator(this).apply {
                setCaptureActivity(PortraitCaptureActivity::class.java)
                setOrientationLocked(true)
                setDesiredBarcodeFormats(IntentIntegrator.QR_CODE)
                setPrompt("Наведите на QR в приложении на ПК")
                setBeepEnabled(false)
                initiateScan()
            }
        }
    }

    override fun onResume() {
        super.onResume()
        val filter = IntentFilter(MicService.ACTION_LEVEL)
        if (Build.VERSION.SDK_INT >= 33) {
            registerReceiver(rx, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            registerReceiver(rx, filter)
        }
        // Мгновенное считывание текущего состояния без ожидания первого аудио-тика
        if (MicService.isRunning) {
            setStatus(MicService.currentState)
        } else {
            setStatus("Остановлен")
        }
    }

    override fun onPause() {
        super.onPause()
        unregisterReceiver(rx)
    }

    @Deprecated("classic onActivityResult for zxing IntentIntegrator")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        val res = IntentIntegrator.parseActivityResult(requestCode, resultCode, data)
        if (res != null && res.contents != null) {
            applyQr(res.contents)
        } else {
            super.onActivityResult(requestCode, resultCode, data)
        }
    }

    /** Парсинг QR десктопа: {"ip","port","token"} → поля + автосохранение */
    private fun applyQr(contents: String) {
        try {
            val o = JSONObject(contents)
            val qip = o.optString("ip", "")
            val qport = o.optInt("port", 59100)
            val qtoken = o.optString("token", "")
            if (qip.isBlank() || qtoken.isBlank()) throw IllegalArgumentException("нет ip/token")

            ip.setText(qip)
            port.setText(qport.toString())
            token.setText(qtoken)

            getPreferences(Context.MODE_PRIVATE).edit()
                .putString("ip", qip)
                .putString("port", qport.toString())
                .putString("token", qtoken)
                .apply()

            // Скрываем ручной ввод, раз параметры успешно подтянуты из QR
            TransitionManager.beginDelayedTransition(rootScroll)
            manualFields.visibility = View.GONE
            toggleManual.text = "Ввести параметры вручную"

            Toast.makeText(this, "Подключено к ПК: $qip:$qport", Toast.LENGTH_SHORT).show()
        } catch (e: Exception) {
            Toast.makeText(this, "Не удалось распознать QR: ${e.message}", Toast.LENGTH_LONG).show()
        }
    }

    private fun applyKeepScreen(on: Boolean) {
        if (on) window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        else window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    }

    private fun audioSourceAt(idx: Int): Int = when (idx) {
        1 -> android.media.MediaRecorder.AudioSource.UNPROCESSED
        2 -> android.media.MediaRecorder.AudioSource.CAMCORDER
        3 -> android.media.MediaRecorder.AudioSource.VOICE_COMMUNICATION
        else -> android.media.MediaRecorder.AudioSource.VOICE_RECOGNITION
    }

    private fun requestPerms() {
        val need = mutableListOf(Manifest.permission.RECORD_AUDIO, Manifest.permission.CAMERA)
        if (Build.VERSION.SDK_INT >= 33) need.add(Manifest.permission.POST_NOTIFICATIONS)
        val missing = need.filter {
            checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED
        }
        if (missing.isNotEmpty()) {
            requestPermissions(missing.toTypedArray(), 1)
        }
    }
}
