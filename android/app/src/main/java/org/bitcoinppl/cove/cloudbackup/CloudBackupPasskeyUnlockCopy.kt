package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupPasskeyUnlockHint

/**
 * The step to take before trying a passkey request that didn't finish again
 *
 * A locked password manager ends the request the same way as a cancel, so the copy after it says
 * what to unlock; the same words as iOS
 */
internal fun CloudBackupPasskeyUnlockHint.retryStep(): String =
    when (this) {
        is CloudBackupPasskeyUnlockHint.Provider -> "If $name is locked, unlock it, then try again."
        CloudBackupPasskeyUnlockHint.PasswordManager ->
            "If your passkey is in a password manager like 1Password, unlock it, then try again."
    }

/** The unlock banner's title, naming what to unlock; the same words as iOS */
internal fun CloudBackupPasskeyUnlockHint.bannerTitle(): String =
    when (this) {
        is CloudBackupPasskeyUnlockHint.Provider -> "Unlock $name"
        CloudBackupPasskeyUnlockHint.PasswordManager -> "Unlock Your Password Manager"
    }

/** The unlock banner's message: the verification didn't finish, and what to unlock first */
internal fun CloudBackupPasskeyUnlockHint.bannerMessage(): String =
    when (this) {
        is CloudBackupPasskeyUnlockHint.Provider ->
            "Verification couldn't use your passkey. If $name is locked, unlock it, then tap Verify Now."
        CloudBackupPasskeyUnlockHint.PasswordManager ->
            "Verification couldn't use your passkey. If it's in a password manager like 1Password, " +
                "unlock it, then tap Verify Now."
    }
