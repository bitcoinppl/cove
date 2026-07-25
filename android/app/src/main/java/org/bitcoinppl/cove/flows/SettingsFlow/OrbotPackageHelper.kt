package org.bitcoinppl.cove.flows.SettingsFlow

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import org.bitcoinppl.cove.Log

private const val ORBOT_PACKAGE = "org.torproject.android"
private const val TAG = "OrbotPackageHelper"

data class OrbotPackage(
    val versionName: String?,
)

object OrbotPackageHelper {
    fun detect(context: Context): OrbotPackage? =
        runCatching {
            val info =
                context.packageManager.getPackageInfo(
                    ORBOT_PACKAGE,
                    PackageManager.PackageInfoFlags.of(0),
                )

            OrbotPackage(info.versionName)
        }.getOrNull()

    /**
     * Returns false when Orbot is missing or the launch was rejected by the system
     */
    fun openOrbot(context: Context): Boolean {
        val launchIntent = context.packageManager.getLaunchIntentForPackage(ORBOT_PACKAGE) ?: return false

        return startActivitySafely(context, launchIntent)
    }

    /**
     * Returns false when neither the store nor a browser could handle the install link
     */
    fun openInstallPage(context: Context): Boolean {
        val marketIntent =
            Intent(Intent.ACTION_VIEW, Uri.parse("market://details?id=$ORBOT_PACKAGE"))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val webIntent =
            Intent(
                Intent.ACTION_VIEW,
                Uri.parse("https://play.google.com/store/apps/details?id=$ORBOT_PACKAGE"),
            ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

        return startActivitySafely(context, marketIntent) || startActivitySafely(context, webIntent)
    }

    // devices without a store, without a browser, or with a restricted profile reject the
    // launch, and an uncaught ActivityNotFoundException or SecurityException would crash the app
    @Suppress("TooGenericExceptionCaught", "SwallowedException")
    private fun startActivitySafely(
        context: Context,
        intent: Intent,
    ): Boolean =
        try {
            context.startActivity(intent)
            true
        } catch (error: Throwable) {
            Log.w(TAG, "unable to start activity for ${intent.data ?: intent.component}", error)
            false
        }
}

/**
 * Orbot install state, re-probed on every resume so installing or removing Orbot while the
 * screen is backgrounded is reflected on return
 */
@Composable
fun rememberOrbotPackage(): State<OrbotPackage?> {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val orbotPackage = remember(context) { mutableStateOf(OrbotPackageHelper.detect(context)) }

    DisposableEffect(lifecycleOwner, context) {
        val observer =
            LifecycleEventObserver { _, event ->
                if (event == Lifecycle.Event.ON_RESUME) {
                    orbotPackage.value = OrbotPackageHelper.detect(context)
                }
            }

        lifecycleOwner.lifecycle.addObserver(observer)

        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    return orbotPackage
}
