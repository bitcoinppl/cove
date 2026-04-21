package org.bitcoinppl.cove.cloudbackup

import android.app.Activity
import android.content.Context
import androidx.activity.result.IntentSenderRequest
import com.google.android.gms.auth.api.identity.AuthorizationRequest
import com.google.android.gms.auth.api.identity.AuthorizationResult
import com.google.android.gms.auth.api.identity.ClearTokenRequest
import com.google.android.gms.auth.api.identity.Identity
import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.Scope
import kotlinx.coroutines.tasks.await

internal class AuthorizationRequiredException(message: String) : Exception(message)

internal class DriveAuthorizationHelper(
    context: Context,
) {
    private val appContext = context.applicationContext
    private val client by lazy { Identity.getAuthorizationClient(appContext) }
    private val requestedScopes = listOf(Scope(DRIVE_APP_DATA_SCOPE))

    suspend fun accessToken(interactive: Boolean): String {
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

    suspend fun clearToken(token: String) {
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

        val activityResult =
            ForegroundUiBridge.launchAuthorization(
                IntentSenderRequest.Builder(pendingIntent.intentSender).build(),
            )

        if (activityResult.resultCode != Activity.RESULT_OK) {
            throw AuthorizationRequiredException("google drive authorization was cancelled")
        }

        val resultIntent =
            activityResult.data
                ?: throw AuthorizationRequiredException("authorization result is missing data")

        return client.getAuthorizationResultFromIntent(resultIntent)
    }

    companion object {
        internal const val DRIVE_APP_DATA_SCOPE =
            "https://www.googleapis.com/auth/drive.appdata"
    }
}
