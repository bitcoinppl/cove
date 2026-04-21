package org.bitcoinppl.cove.cloudbackup

import android.content.Context
import android.os.Looper
import android.util.Base64
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.CreatePublicKeyCredentialResponse
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.PublicKeyCredential
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.NoCredentialException
import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove_core.device.DiscoveredPasskeyResult
import org.bitcoinppl.cove_core.device.PasskeyCredentialPresence
import org.bitcoinppl.cove_core.device.PasskeyException
import org.bitcoinppl.cove_core.device.PasskeyProvider
import org.json.JSONArray
import org.json.JSONObject

class AndroidPasskeyProvider(
    context: Context,
) : PasskeyProvider {
    private val appContext = context.applicationContext
    private val credentialManager by lazy { CredentialManager.create(appContext) }

    override fun createPasskey(
        rpId: String,
        userId: ByteArray,
        challenge: ByteArray,
    ): ByteArray {
        enforceBackgroundThread("createPasskey")
        return runBlocking {
            val activity = ForegroundUiBridge.requireActivity()
            try {
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

                extractCredentialId(registration.registrationResponseJson)
            } catch (error: Exception) {
                throw mapCreateError(error)
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
            val activity = ForegroundUiBridge.requireActivity()
            try {
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
                throw mapGetError(error)
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
            val activity = ForegroundUiBridge.requireActivity()
            try {
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
                throw mapGetError(error)
            }
        }
    }

    override fun isPrfSupported(): Boolean = true

    override fun checkPasskeyPresence(
        rpId: String,
        credentialId: ByteArray,
    ): PasskeyCredentialPresence {
        enforceBackgroundThread("checkPasskeyPresence")
        return runBlocking {
            val activity = ForegroundUiBridge.requireActivity()
            try {
                credentialManager.getCredential(
                    activity,
                    buildGetCredentialRequest(
                        requestJson = buildPresenceCheckRequestJson(rpId, credentialId),
                        preferImmediatelyAvailableCredentials = true,
                    ),
                )
                PasskeyCredentialPresence.PRESENT
            } catch (error: Exception) {
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
            ).toString()

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
            .put("challenge", ByteArray(32).toBase64Url())
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

    private fun extractPrfOutput(responseJson: String): ByteArray {
        val json = JSONObject(responseJson)
        val clientExtensionResults =
            json.optJSONObject("clientExtensionResults")
                ?: json.optJSONObject("response")?.optJSONObject("clientExtensionResults")

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

    private fun mapCreateError(error: Exception): PasskeyException {
        if (error is PasskeyException) {
            return error
        }

        if (error is CreateCredentialException) {
            return when {
                error.javaClass.simpleName.contains("Cancellation", ignoreCase = true) ->
                    PasskeyException.UserCancelled()
                error.javaClass.simpleName.contains("Unsupported", ignoreCase = true) ->
                    PasskeyException.NotSupported(error.message ?: "credential manager is unavailable")
                else ->
                    PasskeyException.CreationFailed(error.message ?: "passkey creation failed")
            }
        }

        return PasskeyException.CreationFailed(error.message ?: "passkey creation failed")
    }

    private fun mapGetError(error: Exception): PasskeyException {
        if (error is PasskeyException) {
            return error
        }

        if (error is NoCredentialException) {
            return PasskeyException.NoCredentialFound()
        }

        if (error is GetCredentialException) {
            return when {
                error.javaClass.simpleName.contains("Cancellation", ignoreCase = true) ->
                    PasskeyException.UserCancelled()
                error.javaClass.simpleName.contains("Unsupported", ignoreCase = true) ->
                    PasskeyException.NotSupported(error.message ?: "credential manager is unavailable")
                else ->
                    PasskeyException.AuthenticationFailed(error.message ?: "passkey authentication failed")
            }
        }

        return PasskeyException.AuthenticationFailed(error.message ?: "passkey authentication failed")
    }

    private fun enforceBackgroundThread(operation: String) {
        check(Looper.myLooper() != Looper.getMainLooper()) {
            "$operation must not run on the main thread"
        }
    }

    private fun ByteArray.toBase64Url(): String =
        Base64.encodeToString(this, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)

    private fun String.fromBase64Url(): ByteArray =
        Base64.decode(this, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
}
