package org.bitcoinppl.cove.cloudbackup

import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialUnsupportedException
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialUnsupportedException
import org.bitcoinppl.cove_core.device.PasskeyException
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.assertThrows
import org.junit.Test

class AndroidPasskeyProviderTest {
    @Test
    fun createRequestJsonRequestsPrfExtension() {
        val request =
            JSONObject(
                buildPasskeyCreateRequestJson(
                    rpId = "covebitcoinwallet.com",
                    userId = byteArrayOf(1, 2, 3),
                    challenge = byteArrayOf(4, 5, 6),
                ),
            )

        val prf = request.getJSONObject("extensions").getJSONObject("prf")

        assertEquals(0, prf.length())
    }

    @Test
    fun registrationPrfValidationAcceptsRootExtensionResults() {
        validatePasskeyRegistrationPrf(
            registrationResponseJson(
                JSONObject()
                    .put("prf", JSONObject().put("enabled", true)),
            ),
        )
    }

    @Test
    fun registrationPrfValidationAcceptsNestedExtensionResults() {
        validatePasskeyRegistrationPrf(
            JSONObject()
                .put(
                    "response",
                    JSONObject()
                        .put(
                            "clientExtensionResults",
                            JSONObject()
                                .put("prf", JSONObject().put("enabled", true)),
                        ),
                ).toString(),
        )
    }

    @Test
    fun registrationPrfValidationRejectsMissingOrDisabledPrf() {
        assertThrows(PasskeyException.PrfUnsupportedProvider::class.java) {
            validatePasskeyRegistrationPrf(registrationResponseJson(JSONObject()))
        }

        assertThrows(PasskeyException.PrfUnsupportedProvider::class.java) {
            validatePasskeyRegistrationPrf(
                registrationResponseJson(
                    JSONObject()
                        .put("prf", JSONObject().put("enabled", false)),
                ),
            )
        }
    }

    @Test
    fun createCredentialExceptionsMapFromTypedAndroidxExceptions() {
        assertTrue(
            mapPasskeyCreateError(CreateCredentialCancellationException())
                is PasskeyException.UserCancelled,
        )
        assertTrue(
            mapPasskeyCreateError(CreateCredentialUnsupportedException())
                is PasskeyException.NotSupported,
        )
    }

    @Test
    fun getCredentialExceptionsMapFromTypedAndroidxExceptions() {
        assertTrue(
            mapPasskeyGetError(GetCredentialCancellationException())
                is PasskeyException.UserCancelled,
        )
        assertTrue(
            mapPasskeyGetError(GetCredentialUnsupportedException())
                is PasskeyException.NotSupported,
        )
    }

    private fun registrationResponseJson(clientExtensionResults: JSONObject): String =
        JSONObject()
            .put("rawId", "AQID")
            .put("clientExtensionResults", clientExtensionResults)
            .toString()
}
