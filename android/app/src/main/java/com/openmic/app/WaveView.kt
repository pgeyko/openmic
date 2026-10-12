package com.openmic.app

import android.content.Context
import android.graphics.*
import android.util.AttributeSet
import android.view.View

/**
 * Профессиональная полоса-волна уровня звука.
 * Плавный градиент (Cyan -> Green), индикация клиппинга, сглаженные столбики.
 */
class WaveView @JvmOverloads constructor(
    ctx: Context, attrs: AttributeSet? = null,
) : View(ctx, attrs) {

    private val cap = 64
    private val buf = ArrayDeque<Float>(cap)

    private val barPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.FILL
    }

    private val clipPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = 0xFFFF5252.toInt() // Красный при клиппинге
        style = Paint.Style.FILL
    }

    private var gradient: LinearGradient? = null
    private var lastHeight = 0f

    fun push(rms: Float) {
        if (buf.size >= cap) buf.removeFirst()
        buf.addLast(rms.coerceIn(0f, 1f))
        postInvalidateOnAnimation()
    }

    fun clear() {
        buf.clear()
        postInvalidateOnAnimation()
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        val hF = h.toFloat()
        if (hF != lastHeight && hF > 0) {
            lastHeight = hF
            // Градиент от акцентного синего к изумрудному
            gradient = LinearGradient(
                0f, hF, 0f, 0f,
                0xFF2979FF.toInt(), 0xFF00E676.toInt(),
                Shader.TileMode.CLAMP
            )
            barPaint.shader = gradient
        }
    }

    override fun onDraw(c: Canvas) {
        super.onDraw(c)
        val w = width.toFloat()
        val h = height.toFloat()

        if (buf.isEmpty()) {
            // Линия тишины в центре
            val centerPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
                color = 0xFF2C2C2C.toInt()
                strokeWidth = 2f
            }
            c.drawLine(0f, h / 2, w, h / 2, centerPaint)
            return
        }

        val n = buf.size
        val step = w / cap
        val barWidth = (step * 0.65f).coerceAtLeast(3f)
        val cornerRadius = barWidth / 2f

        buf.forEachIndexed { i, v ->
            val x = w - (n - i) * step + (step - barWidth) / 2
            val minHeight = 4f
            val barH = (v * (h * 0.9f)).coerceAtLeast(minHeight)
            val top = (h - barH) / 2
            val bottom = top + barH

            val p = if (v >= 0.95f) clipPaint else barPaint
            c.drawRoundRect(x, top, x + barWidth, bottom, cornerRadius, cornerRadius, p)
        }
    }
}
