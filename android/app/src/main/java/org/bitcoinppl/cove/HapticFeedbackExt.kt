package org.bitcoinppl.cove

import android.content.Context
import android.os.Build
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.util.Log
import org.bitcoinppl.cove_core.HapticFeedback

fun HapticFeedback.trigger(context: Context) {
    when (this) {
        HapticFeedback.PROGRESS -> triggerLightVibration(context)
        HapticFeedback.SUCCESS -> triggerSuccessVibration(context)
        HapticFeedback.NONE -> { /* no-op */ }
    }
}

private fun triggerLightVibration(context: Context) {
    try {
        val vibrator = getVibrator(context)
        vibrator?.let {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                it.vibrate(VibrationEffect.createOneShot(50, VibrationEffect.DEFAULT_AMPLITUDE))
            } else {
                @Suppress("DEPRECATION")
                it.vibrate(50)
            }
        }
    } catch (e: Exception) {
        Log.w("HapticFeedback", "Failed to trigger light vibration", e)
    }
}

private fun triggerSuccessVibration(context: Context) {
    try {
        val vibrator = getVibrator(context)
        vibrator?.let {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                it.vibrate(VibrationEffect.createPredefined(VibrationEffect.EFFECT_TICK))
            } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                it.vibrate(VibrationEffect.createOneShot(100, VibrationEffect.DEFAULT_AMPLITUDE))
            } else {
                @Suppress("DEPRECATION")
                it.vibrate(100)
            }
        }
    } catch (e: Exception) {
        Log.w("HapticFeedback", "Failed to trigger success vibration", e)
    }
}

private fun getVibrator(context: Context): Vibrator? =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        val vibratorManager = context.getSystemService(Context.VIBRATOR_MANAGER_SERVICE) as? VibratorManager
        vibratorManager?.defaultVibrator
    } else {
        @Suppress("DEPRECATION")
        context.getSystemService(Context.VIBRATOR_SERVICE) as? Vibrator
    }
