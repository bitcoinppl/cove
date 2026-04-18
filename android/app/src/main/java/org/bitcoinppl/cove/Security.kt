package org.bitcoinppl.cove

import android.content.Context
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import org.bitcoinppl.cove_core.device.DeviceAccess
import org.bitcoinppl.cove_core.device.KeychainAccess
import org.bitcoinppl.cove_core.device.KeychainException
import java.security.GeneralSecurityException
import java.util.TimeZone

class KeychainAccessor(
    context: Context,
) : KeychainAccess {
    private val sharedPreferences: SharedPreferences

    init {
        val useStrongBox = hasStrongBox(context)
        sharedPreferences =
            try {
                createEncryptedPrefs(context, requestStrongBox = useStrongBox)
            } catch (e: GeneralSecurityException) {
                if (!useStrongBox) throw e // not a StrongBox issue, no fallback available
                Log.w("KeychainAccessor", "StrongBox-backed prefs failed, falling back to TEE", e)
                createEncryptedPrefs(context, requestStrongBox = false)
            }
    }

    override fun save(key: String, value: String) {
        val success =
            sharedPreferences
                .edit()
                .putString(key, value)
                .commit()

        if (!success) {
            throw KeychainException.Save()
        }
    }

    override fun get(key: String): String? = sharedPreferences.getString(key, null)

    override fun delete(key: String): Boolean =
        sharedPreferences
            .edit()
            .remove(key)
            .commit()
}

private fun hasStrongBox(context: Context): Boolean =
    context.packageManager.hasSystemFeature(PackageManager.FEATURE_STRONGBOX_KEYSTORE)

private fun createEncryptedPrefs(context: Context, requestStrongBox: Boolean): SharedPreferences {
    val masterKey =
        MasterKey
            .Builder(context)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .setRequestStrongBoxBacked(requestStrongBox)
            .build()

    return EncryptedSharedPreferences.create(
        context,
        "cove_secure_storage",
        masterKey,
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )
}

class DeviceAccessor : DeviceAccess {
    override fun timezone(): String = TimeZone.getDefault().id
}

/** Ref-counts the number of sensitive screens currently on the back stack.
 *  Seed screens call enter() on appear and exit() on dispose. FLAG_SECURE is only
 *  cleared when the count reaches zero, preventing gaps during screen transitions. */
object ScreenSecurity {
    private val count = java.util.concurrent.atomic.AtomicInteger(0)

    val isSensitiveScreen: Boolean
        get() = count.get() > 0

    fun enter() { count.incrementAndGet() }

    fun exit() { count.updateAndGet { if (it > 0) it - 1 else 0 } }
}
