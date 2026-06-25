package org.bitcoinppl.cove

import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageView

internal fun MainActivity.setupPrivacyCover(): View {
    val iconSize = (144 * resources.displayMetrics.density).toInt()

    val imageView =
        ImageView(this).apply {
            setImageResource(R.drawable.ic_launcher_foreground)
            scaleType = ImageView.ScaleType.FIT_CENTER
        }

    val container =
        FrameLayout(this).apply {
            setBackgroundColor(android.graphics.Color.BLACK)
            val params =
                FrameLayout.LayoutParams(iconSize, iconSize).apply {
                    gravity = Gravity.CENTER
                }
            addView(imageView, params)
            visibility = View.GONE
        }

    addContentView(
        container,
        FrameLayout.LayoutParams(
            FrameLayout.LayoutParams.MATCH_PARENT,
            FrameLayout.LayoutParams.MATCH_PARENT,
        ),
    )

    return container
}
