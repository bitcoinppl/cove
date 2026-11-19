package org.bitcoinppl.cove

import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt

/**
 * The biometric classes Cove accepts for unlocking the app and changing its lock
 *
 * Only Class 3 (strong) biometrics qualify. Many phones, such as Samsung Galaxy devices, enroll a
 * camera-based face unlock that Android rates Class 1 or 2 (weak), and a prompt that does not set
 * its allowed authenticators defaults to weak, so it would offer that face unlock beside the
 * fingerprint even though the availability check only counted strong sensors
 */
object BiometricPolicy {
    const val ALLOWED_AUTHENTICATORS = BiometricManager.Authenticators.BIOMETRIC_STRONG

    /** Whether the device has an enrolled biometric that meets the policy */
    fun isAvailable(biometricManager: BiometricManager): Boolean =
        biometricManager.canAuthenticate(ALLOWED_AUTHENTICATORS) == BiometricManager.BIOMETRIC_SUCCESS

    /**
     * Builds prompt info restricted to the policy's authenticators
     *
     * This is the only place that builds a `PromptInfo`; detekt forbids the builder elsewhere so
     * no prompt falls back to weak biometrics
     */
    fun promptInfo(
        title: String,
        subtitle: String,
        negativeButtonText: String,
    ): BiometricPrompt.PromptInfo =
        BiometricPrompt.PromptInfo
            .Builder()
            .setTitle(title)
            .setSubtitle(subtitle)
            .setNegativeButtonText(negativeButtonText)
            .setAllowedAuthenticators(ALLOWED_AUTHENTICATORS)
            .build()
}
