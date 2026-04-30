package org.bitcoinppl.cove.cloudbackup

import android.content.Context
import android.os.Looper
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.CreatePublicKeyCredentialResponse
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.PublicKeyCredential
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.CreateCredentialUnsupportedException
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.GetCredentialUnsupportedException
import androidx.credentials.exceptions.NoCredentialException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove_core.device.DiscoveredPasskeyResult
import org.bitcoinppl.cove_core.device.PasskeyCredentialPresence
import org.bitcoinppl.cove_core.device.PasskeyException
import org.bitcoinppl.cove_core.device.PasskeyProvider
import org.json.JSONArray
import org.json.JSONObject
import java.security.SecureRandom
import java.util.Base64

class AndroidPasskeyProvider(
    context: Context,
) : PasskeyProvider {
    private val appContext = context.applicationContext
    private val credentialManager by lazy { CredentialManager.create(appContext) }
    private val secureRandom = SecureRandom()

    override fun createPasskey(
        rpId: String,
        userId: ByteArray,
        challenge: ByteArray,
    ): ByteArray {
        enforceBackgroundThread("createPasskey")
        return runBlocking {
            try {
                val activity = ForegroundUiBridge.requireActivity()
                val response =
                    credentialManager.createCredential(
                        activity,
                        CreatePublicKeyCredentialRequest(
                            requestJson = buildCreateRequestJson(rpId, userId, challenge),
                        ),
                    )

                val registration =
                    response as? CreatePublicKeyCredentialResponse
                        ?: throw PasskeyException.CreationFailed("unexpected credential response type")

                validatePasskeyRegistrationPrf(registration.registrationResponseJson)
                extractCreatedCredentialId(registration.registrationResponseJson)
            } catch (error: Exception) {
                if (error is CancellationException) throw error
                throw mapPasskeyCreateError(error)
            }
        }
    }

    override fun authenticateWithPrf(
        rpId: String,
        credentialId: ByteArray,
        prfSalt: ByteArray,
        challenge: ByteArray,
    ): ByteArray {
        enforceBackgroundThread("authenticateWithPrf")
        return runBlocking {
            try {
                val activity = ForegroundUiBridge.requireActivity()
                val response =
                    credentialManager.getCredential(
                        activity,
                        buildGetCredentialRequest(
                            requestJson = buildAssertionRequestJson(rpId, credentialId, prfSalt, challenge),
                            preferImmediatelyAvailableCredentials = false,
                        ),
                    )

                val credential =
                    response.credential as? PublicKeyCredential
                        ?: throw PasskeyException.AuthenticationFailed("unexpected credential type")

                extractPrfOutput(credential.authenticationResponseJson)
            } catch (error: Exception) {
                if (error is CancellationException) throw error
                throw mapPasskeyGetError(error)
            }
        }
    }

    override fun discoverAndAuthenticateWithPrf(
        rpId: String,
        prfSalt: ByteArray,
        challenge: ByteArray,
    ): DiscoveredPasskeyResult {
        enforceBackgroundThread("discoverAndAuthenticateWithPrf")
        return runBlocking {
            try {
                val activity = ForegroundUiBridge.requireActivity()
                val response =
                    credentialManager.getCredential(
                        activity,
                        buildGetCredentialRequest(
                            requestJson = buildAssertionRequestJson(rpId, null, prfSalt, challenge),
                            preferImmediatelyAvailableCredentials = false,
                        ),
                    )

                val credential =
                    response.credential as? PublicKeyCredential
                        ?: throw PasskeyException.NoCredentialFound()

                DiscoveredPasskeyResult(
                    prfOutput = extractPrfOutput(credential.authenticationResponseJson),
                    credentialId = extractCredentialId(credential.authenticationResponseJson),
                )
            } catch (error: Exception) {
                if (error is CancellationException) throw error
                throw mapPasskeyGetError(error)
            }
        }
    }

    // prf support is verified lazily when registration and authentication responses are parsed
    override fun isPrfSupported(): Boolean = true

    override fun checkPasskeyPresence(
        rpId: String,
        credentialId: ByteArray,
    ): PasskeyCredentialPresence {
        enforceBackgroundThread("checkPasskeyPresence")
        return runBlocking {
            try {
                val activity = ForegroundUiBridge.requireActivity()
                credentialManager.getCredential(
                    activity,
                    buildGetCredentialRequest(
                        requestJson = buildPresenceCheckRequestJson(rpId, credentialId),
                        preferImmediatelyAvailableCredentials = true,
                    ),
                )
                PasskeyCredentialPresence.PRESENT
            } catch (error: Exception) {
                if (error is CancellationException) throw error
                when (error) {
                    is NoCredentialException -> PasskeyCredentialPresence.MISSING
                    else -> {
                        Log.w("AndroidPasskeyProvider", "presence check was indeterminate", error)
                        PasskeyCredentialPresence.INDETERMINATE
                    }
                }
            }
        }
    }

    private fun buildCreateRequestJson(
        rpId: String,
        userId: ByteArray,
        challenge: ByteArray,
    ): String = buildPasskeyCreateRequestJson(rpId, userId, challenge)

    private fun buildAssertionRequestJson(
        rpId: String,
        credentialId: ByteArray?,
        prfSalt: ByteArray,
        challenge: ByteArray,
    ): String {
        val request =
            JSONObject()
                .put("challenge", challenge.toBase64Url())
                .put("rpId", rpId)
                .put("timeout", 120_000)
                .put("userVerification", "preferred")
                .put(
                    "extensions",
                    JSONObject().put(
                        "prf",
                        JSONObject().put(
                            "eval",
                            JSONObject().put("first", prfSalt.toBase64Url()),
                        ),
                    ),
                )

        credentialId?.let {
            request.put(
                "allowCredentials",
                JSONArray().put(
                    JSONObject()
                        .put("type", "public-key")
                        .put("id", it.toBase64Url()),
                ),
            )
        }

        return request.toString()
    }

    private fun buildPresenceCheckRequestJson(
        rpId: String,
        credentialId: ByteArray,
    ): String =
        JSONObject()
            .put("challenge", randomChallenge().toBase64Url())
            .put("rpId", rpId)
            .put("timeout", 1_000)
            .put(
                "allowCredentials",
                JSONArray().put(
                    JSONObject()
                        .put("type", "public-key")
                        .put("id", credentialId.toBase64Url()),
                ),
            ).toString()

    private fun randomChallenge(): ByteArray =
        ByteArray(32).also(secureRandom::nextBytes)

    private fun buildGetCredentialRequest(
        requestJson: String,
        preferImmediatelyAvailableCredentials: Boolean,
    ): GetCredentialRequest =
        GetCredentialRequest(
            credentialOptions = listOf(GetPublicKeyCredentialOption(requestJson)),
            preferImmediatelyAvailableCredentials = preferImmediatelyAvailableCredentials,
        )

    private fun extractCredentialId(responseJson: String): ByteArray {
        val json = JSONObject(responseJson)
        val rawId = json.optString("rawId").ifBlank { json.optString("id") }
        if (rawId.isBlank()) {
            throw PasskeyException.AuthenticationFailed("credential id was missing from the passkey response")
        }
        return rawId.fromBase64Url()
    }

    private fun extractCreatedCredentialId(responseJson: String): ByteArray {
        val json = JSONObject(responseJson)
        val rawId = json.optString("rawId").ifBlank { json.optString("id") }
        if (rawId.isBlank()) {
            throw PasskeyException.CreationFailed("credential id was missing from the passkey response")
        }
        return rawId.fromBase64Url()
    }

    private fun extractPrfOutput(responseJson: String): ByteArray {
        val json = JSONObject(responseJson)
        val clientExtensionResults =
            json.passkeyClientExtensionResults()

        val first =
            clientExtensionResults
                ?.optJSONObject("prf")
                ?.optJSONObject("results")
                ?.optString("first")

        if (first.isNullOrBlank()) {
            throw PasskeyException.PrfUnsupportedProvider()
        }

        val prfOutput = first.fromBase64Url()
        if (prfOutput.size < 32) {
            throw PasskeyException.PrfUnsupportedProvider()
        }

        return prfOutput.copyOf(32)
    }

    private fun enforceBackgroundThread(operation: String) {
        check(Looper.myLooper() != Looper.getMainLooper()) {
            "$operation must not run on the main thread"
        }
    }

}

internal fun mapPasskeyCreateError(error: Exception): PasskeyException =
    when (error) {
        is PasskeyException -> error
        is CreateCredentialCancellationException -> PasskeyException.UserCancelled()
        is CreateCredentialUnsupportedException ->
            PasskeyException.NotSupported(error.message ?: "credential manager is unavailable")
        is CreateCredentialException ->
            PasskeyException.CreationFailed(error.message ?: "passkey creation failed")
        else -> PasskeyException.CreationFailed(error.message ?: "passkey creation failed")
    }

internal fun mapPasskeyGetError(error: Exception): PasskeyException =
    when (error) {
        is PasskeyException -> error
        is NoCredentialException -> PasskeyException.NoCredentialFound()
        is GetCredentialCancellationException -> PasskeyException.UserCancelled()
        is GetCredentialUnsupportedException ->
            PasskeyException.NotSupported(error.message ?: "credential manager is unavailable")
        is GetCredentialException ->
            PasskeyException.AuthenticationFailed(error.message ?: "passkey authentication failed")
        else -> PasskeyException.AuthenticationFailed(error.message ?: "passkey authentication failed")
    }

internal fun buildPasskeyCreateRequestJson(
    rpId: String,
    userId: ByteArray,
    challenge: ByteArray,
): String =
    JSONObject()
        .put("challenge", challenge.toBase64Url())
        .put(
            "rp",
            JSONObject()
                .put("id", rpId)
                .put("name", "Cove Wallet"),
        ).put(
            "user",
            JSONObject()
                .put("id", userId.toBase64Url())
                .put("name", "cloud-backup@covebitcoinwallet.com")
                .put("displayName", "Cove Wallet Backup"),
        ).put(
            "pubKeyCredParams",
            JSONArray()
                .put(JSONObject().put("type", "public-key").put("alg", -7))
                .put(JSONObject().put("type", "public-key").put("alg", -257)),
        ).put("timeout", 120_000)
        .put("attestation", "none")
        .put(
            "authenticatorSelection",
            JSONObject()
                .put("residentKey", "required")
                .put("userVerification", "preferred"),
        ).put(
            "extensions",
            JSONObject().put("prf", JSONObject()),
        ).toString()

internal fun validatePasskeyRegistrationPrf(responseJson: String) {
    val prf =
        JSONObject(responseJson)
            .passkeyClientExtensionResults()
            ?.optJSONObject("prf")

    if (prf?.optBoolean("enabled", false) != true) {
        throw PasskeyException.PrfUnsupportedProvider()
    }
}

private fun JSONObject.passkeyClientExtensionResults(): JSONObject? =
    optJSONObject("clientExtensionResults")
        ?: optJSONObject("response")?.optJSONObject("clientExtensionResults")

private fun ByteArray.toBase64Url(): String =
    Base64.getUrlEncoder().withoutPadding().encodeToString(this)

private fun String.fromBase64Url(): ByteArray =
    Base64.getUrlDecoder().decode(this)
