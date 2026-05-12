package org.bitcoinppl.cove.flows.SettingsFlow

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri

private const val ORBOT_PACKAGE = "org.torproject.android"

object OrbotPackageHelper {
    fun detect(context: Context): Pair<OrbotStatus, String?> =
        runCatching {
            val info =
                context.packageManager.getPackageInfo(
                    ORBOT_PACKAGE,
                    PackageManager.PackageInfoFlags.of(0),
                )
            OrbotStatus.Detected to info.versionName
        }.getOrElse {
            OrbotStatus.NotDetected to null
        }

    fun openOrbot(context: Context): Boolean {
        val launchIntent = context.packageManager.getLaunchIntentForPackage(ORBOT_PACKAGE) ?: return false
        context.startActivity(launchIntent)
        return true
    }

    fun openInstallPage(context: Context) {
        val marketIntent =
            Intent(Intent.ACTION_VIEW, Uri.parse("market://details?id=$ORBOT_PACKAGE"))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

        val webIntent =
            Intent(
                Intent.ACTION_VIEW,
                Uri.parse("https://play.google.com/store/apps/details?id=$ORBOT_PACKAGE"),
            ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

        runCatching { context.startActivity(marketIntent) }
            .recover { context.startActivity(webIntent) }
    }
}
