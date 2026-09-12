package org.bitcoinppl.cove.cloudbackup

import android.os.Bundle
import androidx.credentials.CustomCredential
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialInterruptedException
import androidx.credentials.exceptions.CreateCredentialUnsupportedException
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialInterruptedException
import androidx.credentials.exceptions.GetCredentialUnsupportedException
import androidx.credentials.exceptions.NoCredentialException
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
    fun createRequestJsonRequiresUserVerificationAndPreservesFields() {
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

        assertEquals("BAUG", request.getString("challenge"))

        val rp = request.getJSONObject("rp")
        assertEquals("covebitcoinwallet.com", rp.getString("id"))
        assertEquals("Cove Cloud Backup", rp.getString("name"))

        val user = request.getJSONObject("user")
        assertEquals("AQID", user.getString("id"))
        assertEquals("test@example.com", user.getString("name"))
        assertEquals("Test User", user.getString("displayName"))

        val pubKeyCredParams = request.getJSONArray("pubKeyCredParams")
        assertEquals(2, pubKeyCredParams.length())
        assertEquals("public-key", pubKeyCredParams.getJSONObject(0).getString("type"))
        assertEquals(-7, pubKeyCredParams.getJSONObject(0).getInt("alg"))
        assertEquals("public-key", pubKeyCredParams.getJSONObject(1).getString("type"))
        assertEquals(-257, pubKeyCredParams.getJSONObject(1).getInt("alg"))

        assertEquals("none", request.getString("attestation"))
        assertEquals(
            "required",
            request.getJSONObject("authenticatorSelection").getString("residentKey"),
        )
        assertEquals(
            "required",
            request.getJSONObject("authenticatorSelection").getString("userVerification"),
        )

        val prf = request.getJSONObject("extensions").getJSONObject("prf")

        assertEquals(0, prf.length())
        assertTrue(!request.has("timeout"))
    }

    @Test
    fun assertionRequestJsonRequiresUserVerificationAndPreservesFields() {
        val request =
            JSONObject(
                buildPasskeyAssertionRequestJson(
                    rpId = "covebitcoinwallet.com",
                    credentialId = byteArrayOf(1, 2, 3),
                    prfSalt = byteArrayOf(4, 5, 6),
                    challenge = byteArrayOf(7, 8, 9),
                ),
            )

        assertEquals("BwgJ", request.getString("challenge"))
        assertEquals("covebitcoinwallet.com", request.getString("rpId"))
        assertEquals("required", request.getString("userVerification"))

        val prf = request.getJSONObject("extensions").getJSONObject("prf")
        assertEquals("BAUG", prf.getJSONObject("eval").getString("first"))

        val allowCredentials = request.getJSONArray("allowCredentials")
        assertEquals(1, allowCredentials.length())
        assertEquals("public-key", allowCredentials.getJSONObject(0).getString("type"))
        assertEquals("AQID", allowCredentials.getJSONObject(0).getString("id"))

        assertTrue(!request.has("timeout"))
    }

    @Test
    fun discoveryRejectsUnexpectedCredentialTypeAsAuthenticationFailure() {
        val unexpectedCredential = CustomCredential("unexpected", Bundle())
        val error =
            assertThrows(PasskeyException.RequestFailed::class.java) {
                requireDiscoveredPublicKeyCredential(unexpectedCredential)
            }

        assertEquals(PasskeyOperation.DISCOVER_ASSERTION, error.operation)
        assertEquals(PasskeyFailureReason.UnexpectedCredentialType, error.reason)
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

        val foregroundTimeout = mapPasskeyCreateError(
            ForegroundAuthorizationTimeoutException("foreground activity was unavailable"),
        )
        assertTrue(foregroundTimeout is PasskeyException.RequestFailed)
        assertEquals(
            PasskeyFailureReason.TimedOut,
            (foregroundTimeout as PasskeyException.RequestFailed).reason,
        )

        val timedOut = mapPasskeyCreateError(CreatePublicKeyCredentialDomException(TimeoutError()))
        assertTrue(timedOut is PasskeyException.RequestFailed)
        assertEquals(PasskeyOperation.REGISTRATION, (timedOut as PasskeyException.RequestFailed).operation)
        assertEquals(PasskeyFailureReason.TimedOut, timedOut.reason)

        val localizedDiagnostic = mapPasskeyCreateError(Exception("RP ID cannot be validated."))
        assertTrue(localizedDiagnostic is PasskeyException.RequestFailed)
        assertTrue(
            (localizedDiagnostic as PasskeyException.RequestFailed).reason
                is PasskeyFailureReason.Unknown,
        )
        assertEquals(
            PasskeyFailureReason.Unknown("passkey creation failed"),
            localizedDiagnostic.reason,
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
        assertTrue(
            mapPasskeyGetError(NoCredentialException())
                is PasskeyException.NoCredentialFound,
        )

        val interrupted = mapPasskeyGetError(
            GetCredentialInterruptedException(),
            PasskeyOperation.DISCOVER_ASSERTION,
        )
        assertTrue(interrupted is PasskeyException.RequestFailed)
        assertEquals(PasskeyOperation.DISCOVER_ASSERTION, (interrupted as PasskeyException.RequestFailed).operation)
        assertEquals(PasskeyFailureReason.Interrupted, interrupted.reason)

        val foregroundTimeout = mapPasskeyGetError(
            ForegroundAuthorizationTimeoutException("foreground activity was unavailable"),
            PasskeyOperation.DISCOVER_ASSERTION,
        )
        assertTrue(foregroundTimeout is PasskeyException.RequestFailed)
        assertEquals(
            PasskeyFailureReason.TimedOut,
            (foregroundTimeout as PasskeyException.RequestFailed).reason,
        )

        assertGetCredentialDomExceptionMappings()
    }

    private fun assertGetCredentialDomExceptionMappings() {
        val notAllowed = mapPasskeyGetError(
            GetPublicKeyCredentialDomException(NotAllowedError()),
            PasskeyOperation.DISCOVER_ASSERTION,
        )
        assertTrue(notAllowed is PasskeyException.RequestFailed)
        assertEquals(PasskeyOperation.DISCOVER_ASSERTION, (notAllowed as PasskeyException.RequestFailed).operation)
        assertEquals(
            PasskeyFailureReason.PlatformAuthorizationFailedAfterPresentation,
            notAllowed.reason,
        )

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

        val diagnostic = mapPasskeyGetError(
            Exception("credential provider diagnostic"),
            PasskeyOperation.DISCOVER_ASSERTION,
        )
        assertTrue(diagnostic is PasskeyException.RequestFailed)
        assertEquals(
            PasskeyOperation.DISCOVER_ASSERTION,
            (diagnostic as PasskeyException.RequestFailed).operation,
        )
        assertEquals(
            PasskeyFailureReason.Unknown("passkey authentication failed"),
            diagnostic.reason,
        )
    }

    private fun registrationResponseJson(clientExtensionResults: JSONObject): String =
        JSONObject()
            .put("rawId", "AQID")
            .put("clientExtensionResults", clientExtensionResults)
            .toString()
}
