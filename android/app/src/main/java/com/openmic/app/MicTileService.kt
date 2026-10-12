package com.openmic.app

import android.content.Context
import android.content.Intent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService

/**
 * Плитка быстрого переключения в шторке уведомлений Android.
 * Одно нажатие — запуск или остановка микрофона без открытия приложения.
 */
class MicTileService : TileService() {

    override fun onStartListening() {
        super.onStartListening()
        updateTileState()
    }

    override fun onClick() {
        super.onClick()
        val prefs = getSharedPreferences("MainActivity", Context.MODE_PRIVATE)
        val host = prefs.getString("ip", "").orEmpty()
        val port = prefs.getString("port", "59100")?.toIntOrNull() ?: 59100
        val token = prefs.getString("token", "").orEmpty()

        if (MicService.isRunning) {
            startService(Intent(this, MicService::class.java).setAction("STOP"))
        } else {
            if (host.isNotBlank() && token.isNotBlank()) {
                val svc = Intent(this, MicService::class.java).apply {
                    putExtra("host", host)
                    putExtra("port", port)
                    putExtra("token", token)
                    putExtra("audioSource", prefs.getInt("sourceIdx", 0))
                    putExtra("ns", prefs.getBoolean("ns", false))
                    putExtra("monitor", prefs.getBoolean("monitor", false))
                    putExtra("monitorVol", prefs.getInt("monitorVol", 70))
                }
                if (Build.VERSION.SDK_INT >= 26) {
                    startForegroundService(svc)
                } else {
                    startService(svc)
                }
            } else {
                // Если нет настроек сопряжения — открываем Activity
                val appIntent = Intent(this, MainActivity::class.java).apply {
                    flags = Intent.FLAG_ACTIVITY_NEW_TASK
                }
                startActivityAndCollapse(appIntent)
                return
            }
        }
        updateTileState()
    }

    private fun updateTileState() {
        val tile = qsTile ?: return
        if (MicService.isRunning) {
            tile.state = Tile.STATE_ACTIVE
            tile.label = "OpenMic: Эфир"
        } else {
            tile.state = Tile.STATE_INACTIVE
            tile.label = "OpenMic"
        }
        tile.updateTile()
    }
}
