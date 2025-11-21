package org.bitcoinppl.cove

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import org.bitcoinppl.cove_core.device.DeviceAccess
import org.bitcoinppl.cove_core.device.KeychainAccess
import org.bitcoinppl.cove_core.device.KeychainException
import java.util.TimeZone

class KeychainAccessor(
    context: Context,
) : KeychainAccess {
    private val sharedPreferences: SharedPreferences

    init {
        // create or retrieve the master key for encryption
        val masterKey =
            MasterKey
                .Builder(context)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build()

        // create encrypted shared preferences
        sharedPreferences =
            EncryptedSharedPreferences.create(
                context,
                "cove_secure_storage",
                masterKey,
                EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
            )
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

class DeviceAccessor : DeviceAccess {
    override fun timezone(): String = TimeZone.getDefault().id
}
