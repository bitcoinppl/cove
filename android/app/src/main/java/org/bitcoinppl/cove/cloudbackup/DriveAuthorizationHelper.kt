package org.bitcoinppl.cove.cloudbackup

import android.app.Activity
import android.content.Context
import android.content.Intent
import androidx.activity.result.IntentSenderRequest
import com.google.android.gms.auth.api.identity.AuthorizationRequest
import com.google.android.gms.auth.api.identity.AuthorizationResult
import com.google.android.gms.auth.api.identity.ClearTokenRequest
import com.google.android.gms.auth.api.identity.Identity
import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.Scope
import kotlinx.coroutines.tasks.await

internal class AuthorizationRequiredException(
    message: String,
    cause: Throwable? = null,
) : Exception(message, cause)

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
            ?: throw IllegalStateException("drive authorization succeeded but returned a blank access token")
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

        val activityResult =
            ForegroundUiBridge.launchAuthorization(
                IntentSenderRequest.Builder(pendingIntent.intentSender).build(),
            )

        if (activityResult.resultCode != Activity.RESULT_OK) {
            throw AuthorizationRequiredException("google drive authorization was cancelled")
        }

        val resultIntent =
            activityResult.data ?: Intent()

        return try {
            client.getAuthorizationResultFromIntent(resultIntent)
        } catch (error: ApiException) {
            throw AuthorizationRequiredException("google drive authorization result could not be parsed", error)
        } catch (error: RuntimeException) {
            throw AuthorizationRequiredException("google drive authorization result could not be parsed", error)
        }
    }

    companion object {
        internal const val DRIVE_APP_DATA_SCOPE =
            "https://www.googleapis.com/auth/drive.appdata"
    }
}
