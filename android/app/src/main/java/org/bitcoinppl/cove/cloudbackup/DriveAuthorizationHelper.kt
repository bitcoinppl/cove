package org.bitcoinppl.cove.cloudbackup

import android.content.Context
import android.content.Intent
import androidx.activity.result.IntentSenderRequest
import com.google.android.gms.auth.api.identity.AuthorizationRequest
import com.google.android.gms.auth.api.identity.AuthorizationResult
import com.google.android.gms.auth.api.identity.ClearTokenRequest
import com.google.android.gms.auth.api.identity.Identity
import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.Scope
import com.google.android.gms.tasks.Task
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.suspendCancellableCoroutine

internal class AuthorizationRequiredException(message: String) : Exception(message)

internal interface DriveAuthorization {
    suspend fun accessToken(interactive: Boolean): String

    suspend fun clearToken(token: String)
}

internal class DriveAuthorizationHelper(
    context: Context,
) : DriveAuthorization {
    private val appContext = context.applicationContext
    private val client by lazy { Identity.getAuthorizationClient(appContext) }
    private val requestedScopes = listOf(Scope(DRIVE_APP_DATA_SCOPE))

    override suspend fun accessToken(interactive: Boolean): String {
        val authorizationResult = client
            .authorize(
                AuthorizationRequest
                    .builder()
                    .setRequestedScopes(requestedScopes)
                    .build(),
            ).await()

        val resolved = resolveIfNeeded(authorizationResult, interactive)
        return resolved.accessToken?.takeIf { it.isNotBlank() }
            ?: throw ApiException(com.google.android.gms.common.api.Status.RESULT_INTERNAL_ERROR)
    }

    override suspend fun clearToken(token: String) {
        client
            .clearToken(
                ClearTokenRequest
                    .builder()
                    .setToken(token)
                    .build(),
            ).await()
    }

    private suspend fun resolveIfNeeded(
        authorizationResult: AuthorizationResult,
        interactive: Boolean,
    ): AuthorizationResult {
        if (!authorizationResult.hasResolution()) {
            return authorizationResult
        }

        if (!interactive) {
            throw AuthorizationRequiredException("google drive authorization requires foreground consent")
        }

        val pendingIntent =
            authorizationResult.pendingIntent
                ?: throw AuthorizationRequiredException("authorization resolution is missing a pending intent")

        val resultIntent =
            ForegroundUiBridge.launchAuthorization(
                IntentSenderRequest.Builder(pendingIntent.intentSender).build(),
            ) ?: Intent()

        return client.getAuthorizationResultFromIntent(resultIntent)
    }

    private suspend fun <T> Task<T>.await(): T =
        suspendCancellableCoroutine { continuation ->
            addOnSuccessListener { result ->
                continuation.resume(result)
            }
            addOnFailureListener { error ->
                continuation.resumeWithException(error)
            }
            addOnCanceledListener {
                continuation.resumeWithException(
                    ApiException(com.google.android.gms.common.api.Status.RESULT_CANCELED),
                )
            }
        }

    companion object {
        internal const val DRIVE_APP_DATA_SCOPE =
            "https://www.googleapis.com/auth/drive.appdata"
    }
}
