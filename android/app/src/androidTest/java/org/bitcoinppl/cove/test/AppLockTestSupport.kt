package org.bitcoinppl.cove.test

import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.AuthType
import org.bitcoinppl.cove_core.BootstrapStep
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.bootstrapProgress

/**
 * Returns the app lock to a fresh install's state: no PIN, decoy PIN, wipe data PIN or biometric
 * unlock, the main wallets showing, and the app unlocked
 *
 * Each test runs in its own process, but every test shares the app's database, so a lock one test
 * leaves behind makes later tests that refuse to run over an existing app lock skip themselves.
 * Call it from `@After` so it also runs when the test fails
 */
fun restoreNoAppLock() {
    // a process that never finished bootstrap never opened the database, so it changed no lock
    if (bootstrapProgress() != BootstrapStep.COMPLETE) return

    var isInDecoyMode = true
    var isLocked = true
    InstrumentationRegistry.getInstrumentation().runOnMainSync {
        val auth = AuthManager.getInstance()

        // a main PIN unlock leaves decoy mode, which removing the PINs alone would not
        if (auth.isInDecoyMode()) auth.switchToMainMode()

        auth.dispatch(AuthManagerAction.DisableDecoyPin)
        auth.dispatch(AuthManagerAction.DisableWipeDataPin)
        auth.dispatch(AuthManagerAction.DisablePin)
        auth.dispatch(AuthManagerAction.DisableBiometric)
        auth.unlockIfAuthDisabled()

        isInDecoyMode = auth.isInDecoyMode()
        isLocked = auth.isLocked
    }

    val authType = Database().use { it.globalConfig().use { config -> config.authType() } }
    check(authType == AuthType.NONE && !isInDecoyMode && !isLocked) {
        "the app lock is still set up after the test: authType=$authType decoy=$isInDecoyMode locked=$isLocked"
    }
}
