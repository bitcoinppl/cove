package org.bitcoinppl.cove.cloudbackup

import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialInterruptedException
import androidx.credentials.exceptions.CreateCredentialUnsupportedException
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialInterruptedException
import androidx.credentials.exceptions.GetCredentialUnsupportedException
import androidx.credentials.exceptions.domerrors.DataError
import androidx.credentials.exceptions.domerrors.NotAllowedError
import androidx.credentials.exceptions.domerrors.TimeoutError
import androidx.credentials.exceptions.publickeycredential.CreatePublicKeyCredentialDomException
import androidx.credentials.exceptions.publickeycredential.GetPublicKeyCredentialDomException
import org.bitcoinppl.cove_core.device.PasskeyException
import org.bitcoinppl.cove_core.device.PasskeyFailureReason
import org.bitcoinppl.cove_core.device.PasskeyOperation
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

        val interrupted = mapPasskeyCreateError(CreateCredentialInterruptedException())
        assertTrue(interrupted is PasskeyException.RequestFailed)
        assertEquals(PasskeyOperation.REGISTRATION, (interrupted as PasskeyException.RequestFailed).operation)
        assertEquals(PasskeyFailureReason.Interrupted, interrupted.reason)

        val timedOut = mapPasskeyCreateError(CreatePublicKeyCredentialDomException(TimeoutError()))
        assertTrue(timedOut is PasskeyException.RequestFailed)
        assertEquals(PasskeyOperation.REGISTRATION, (timedOut as PasskeyException.RequestFailed).operation)
        assertEquals(PasskeyFailureReason.TimedOut, timedOut.reason)
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

        val interrupted = mapPasskeyGetError(
            GetCredentialInterruptedException(),
            PasskeyOperation.DISCOVER_ASSERTION,
        )
        assertTrue(interrupted is PasskeyException.RequestFailed)
        assertEquals(PasskeyOperation.DISCOVER_ASSERTION, (interrupted as PasskeyException.RequestFailed).operation)
        assertEquals(PasskeyFailureReason.Interrupted, interrupted.reason)

        val notAllowed = mapPasskeyGetError(
            GetPublicKeyCredentialDomException(NotAllowedError()),
            PasskeyOperation.DISCOVER_ASSERTION,
        )
        assertTrue(notAllowed is PasskeyException.RequestFailed)
        assertEquals(PasskeyOperation.DISCOVER_ASSERTION, (notAllowed as PasskeyException.RequestFailed).operation)
        assertEquals(PasskeyFailureReason.PlatformAuthorizationFailed, notAllowed.reason)

        val dataError = mapPasskeyGetError(GetPublicKeyCredentialDomException(DataError()))
        assertTrue(dataError is PasskeyException.RequestFailed)
        val reason = (dataError as PasskeyException.RequestFailed).reason
        assertTrue(reason is PasskeyFailureReason.Unknown)
        assertTrue((reason as PasskeyFailureReason.Unknown).diagnosticMessage.contains("passkey DOM error"))
    }

    private fun registrationResponseJson(clientExtensionResults: JSONObject): String =
        JSONObject()
            .put("rawId", "AQID")
            .put("clientExtensionResults", clientExtensionResults)
            .toString()
}
