package org.bitcoinppl.cove.cloudbackup

import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialInterruptedException
import androidx.credentials.exceptions.CreateCredentialUnsupportedException
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialInterruptedException
import androidx.credentials.exceptions.GetCredentialUnsupportedException
import androidx.credentials.exceptions.domerrors.DataError
import androidx.credentials.exceptions.domerrors.InvalidStateError
import androidx.credentials.exceptions.domerrors.NotAllowedError
import androidx.credentials.exceptions.domerrors.SecurityError
import androidx.credentials.exceptions.domerrors.TimeoutError
import androidx.credentials.exceptions.publickeycredential.CreatePublicKeyCredentialDomException
import androidx.credentials.exceptions.publickeycredential.GetPublicKeyCredentialDomException
import org.bitcoinppl.cove_core.device.PasskeyException
import org.bitcoinppl.cove_core.device.PasskeyFailureReason
import org.bitcoinppl.cove_core.device.PasskeyOperation
import org.bitcoinppl.cove_core.device.PasskeyRegistrationUser
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
                    challenge = byteArrayOf(4, 5, 6),
                    user =
                        PasskeyRegistrationUser(
                            id = byteArrayOf(1, 2, 3),
                            name = "test@example.com",
                            displayName = "Test User",
                        ),
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

        val rpIdValidation = mapPasskeyCreateError(Exception("RP ID cannot be validated."))
        assertTrue(rpIdValidation is PasskeyException.RequestFailed)
        assertEquals(
            PasskeyFailureReason.DeviceNotConfigured,
            (rpIdValidation as PasskeyException.RequestFailed).reason,
        )

        val createSecurityError = mapPasskeyCreateError(CreatePublicKeyCredentialDomException(SecurityError()))
        assertTrue(createSecurityError is PasskeyException.RequestFailed)
        assertEquals(
            PasskeyFailureReason.DeviceNotConfigured,
            (createSecurityError as PasskeyException.RequestFailed).reason,
        )

        val createDataError = mapPasskeyCreateError(CreatePublicKeyCredentialDomException(DataError()))
        assertTrue(createDataError is PasskeyException.RequestFailed)
        assertEquals(
            PasskeyFailureReason.DeviceNotConfigured,
            (createDataError as PasskeyException.RequestFailed).reason,
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

        val securityError = mapPasskeyGetError(GetPublicKeyCredentialDomException(SecurityError()))
        assertTrue(securityError is PasskeyException.RequestFailed)
        assertEquals(
            PasskeyFailureReason.ProviderConfiguration,
            (securityError as PasskeyException.RequestFailed).reason,
        )

        val invalidState =
            mapPasskeyGetError(GetPublicKeyCredentialDomException(InvalidStateError()))
        assertTrue(invalidState is PasskeyException.RequestFailed)
        assertEquals(
            PasskeyFailureReason.InvalidResponse,
            (invalidState as PasskeyException.RequestFailed).reason,
        )

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
