package com.openmic.app

import java.io.BufferedReader
import java.io.InputStreamReader
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.nio.ByteBuffer
import java.nio.ByteOrder
import org.json.JSONObject

/**
 * TCP-клиент протокола OpenMic v1 (см. docs/TZ.md §5 и openmic-proto):
 * handshake JSON-строкой, затем фреймы [u32 LE len][PCM S16LE mono].
 */
class NetClient(
    private val host: String,
    private val port: Int,
    private val token: String,
) {
    private var socket: Socket? = null
    private var out: OutputStream? = null
    private val lenBuf = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)

    /** S→C control: вызывается из фонового треда-читателя. */
    var onControl: ((type: String, value: Boolean) -> Unit)? = null
    private var readerThread: Thread? = null

    fun connect() {
        val s = Socket()
        s.tcpNoDelay = true
        android.util.Log.i(
            "OpenMic",
            "tcp dial from ${Thread.currentThread().name}#${Thread.currentThread().id}"
        )
        s.connect(InetSocketAddress(host, port), 5000)
        s.soTimeout = 5000
        val hello = JSONObject()
            .put("hello", "openmic/1")
            .put("token", token)
            .toString() + "\n"
        s.getOutputStream().write(hello.toByteArray())
        s.getOutputStream().flush()
        val reader = BufferedReader(InputStreamReader(s.getInputStream()))
        try {
            val welcome = JSONObject(reader.readLine() ?: "")
            if (!welcome.optBoolean("ok", false)) {
                throw IllegalStateException("handshake rejected: $welcome")
            }
        } catch (e: Exception) {
            try { s.close() } catch (_: Exception) {}
            throw e
        }
        // Короткий таймаут: readLine периодически просыпается и видит
        // закрытый сокет — зависших reader-тредов не остаётся.
        s.soTimeout = 2000
        socket = s
        out = s.getOutputStream()
        android.util.Log.i("OpenMic", "tcp connected #${s.hashCode()}")
        // Читатель S→C (десктоп → телефон): {"type":"ns","value":true}.
        // Бинарь пишет только телефон, читает только демон — конфликта нет.
        // Предыдущий читатель (после close) гарантированно мёртв: см. close().
        val cb = onControl
        if (cb != null) {
            readerThread = Thread({
                try {
                    while (!Thread.currentThread().isInterrupted) {
                        val line = try {
                            reader.readLine()
                        } catch (_: java.net.SocketTimeoutException) {
                            continue
                        } ?: break
                        try {
                            val msg = JSONObject(line)
                            if (msg.optString("type") == "ns") {
                                android.util.Log.i(
                                    "OpenMic",
                                    "ctl-recv seq=${msg.optLong("seq", -1)} tid=${Thread.currentThread().id}"
                                )
                                cb("ns", msg.optBoolean("value", false))
                            }
                        } catch (_: Exception) { /* мусор пропускаем */ }
                    }
                } catch (_: Exception) { /* сокет закрыт */ }
            }, "openmic-ctl").apply { isDaemon = true; start() }
        }
    }

    fun sendFrame(pcm: ByteArray, bytes: Int) {
        val o = out ?: throw IllegalStateException("not connected")
        lenBuf.clear()
        lenBuf.putInt(bytes)
        o.write(lenBuf.array())
        o.write(pcm, 0, bytes)
        o.flush() // гарантируем немедленный уход в сеть без накопления задержки
    }

    fun close() {
        val id = socket?.hashCode()
        try { out?.flush() } catch (_: Exception) {}
        try { socket?.close() } catch (_: Exception) {}
        socket = null
        out = null
        android.util.Log.i("OpenMic", "tcp closed #$id")
        readerThread?.interrupt()
        try {
            readerThread?.join(500)
        } catch (_: Exception) {}
        readerThread = null
    }
}
