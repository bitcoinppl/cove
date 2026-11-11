package org.bitcoinppl.cove.cloudbackup

import androidx.credentials.exceptions.GetCredentialCustomException
import androidx.credentials.exceptions.GetCredentialUnknownException
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import org.junit.runner.RunWith

/** A passkey failure log goes through Rust's one allowlist, so only a known credential type shows */
@RunWith(AndroidJUnit4::class)
class PasskeyFailureLogTest {
    @Test
    fun onlyAnAllowlistedCredentialTypeReachesTheLog() {
        val listed =
            passkeyFailureLogMessage(
                "presence check was indeterminate",
                GetCredentialUnknownException("alice@example.com"),
            )
        val unlisted =
            passkeyFailureLogMessage(
                "presence check was indeterminate",
                GetCredentialCustomException("com.example.passkey", "alice@example.com"),
            )

        assertEquals(
            "presence check was indeterminate error_domain=android.credentials.GetCredentialException.TYPE_UNKNOWN",
            listed,
        )
        assertEquals("presence check was indeterminate error_domain=unlisted", unlisted)
        for (line in listOf(listed, unlisted)) {
            assertFalse(line, line.contains("alice") || line.contains("com.example.passkey"))
        }
    }
}
