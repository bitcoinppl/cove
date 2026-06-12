package org.bitcoinppl.cove.cloudbackup

import android.accounts.Account
import android.app.Activity
import android.content.Context
import androidx.activity.result.IntentSenderRequest
import com.google.android.gms.auth.api.identity.AuthorizationRequest
import com.google.android.gms.auth.api.identity.AuthorizationResult
import com.google.android.gms.auth.api.identity.ClearTokenRequest
import com.google.android.gms.auth.api.identity.Identity
import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.CommonStatusCodes
import com.google.android.gms.common.api.Scope
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.tasks.await

internal class AuthorizationRequiredException(
    message: String,
    cause: Throwable? = null,
) : Exception(message, cause)

internal data class DriveAccountIdentity(
    val id: String?,
    val email: String?,
) {
    init {
        require(!id.isNullOrBlank() || !email.isNullOrBlank()) {
            "drive account identity requires id or email"
        }
    }

    val normalizedEmail: String?
        get() = email?.trim()?.lowercase()?.takeIf(String::isNotBlank)

    fun matches(other: DriveAccountIdentity): Boolean {
        if (!id.isNullOrBlank() && !other.id.isNullOrBlank()) {
            return id == other.id
        }

        return normalizedEmail != null && normalizedEmail == other.normalizedEmail
    }

    fun androidAccount(): Account? =
        normalizedEmail?.let { Account(it, GOOGLE_ACCOUNT_TYPE) }

    companion object {
        private const val GOOGLE_ACCOUNT_TYPE = "com.google"

        fun fromAuthorizationResult(result: AuthorizationResult): DriveAccountIdentity? {
            val account = result.toGoogleSignInAccount()
            val id = account?.id?.takeIf(String::isNotBlank)
            val email = account?.email?.takeIf(String::isNotBlank)

            return if (id == null && email == null) {
                null
            } else {
                DriveAccountIdentity(id = id, email = email)
            }
        }
    }
}

internal data class DriveAccessToken(
    val token: String,
    val account: DriveAccountIdentity,
)

internal sealed class DriveAccountBindingException(
    message: String,
) : Exception(message) {
    class MissingIdentity :
        DriveAccountBindingException("google drive account identity is unavailable")

    class Mismatch :
        DriveAccountBindingException("google drive account does not match the account selected for Cloud Backup")
}

internal interface DriveAccountBindingStore {
    fun selectedIdentity(): DriveAccountIdentity?

    fun bindIdentity(identity: DriveAccountIdentity)
}

internal class SharedPreferencesDriveAccountBindingStore(
    context: Context,
) : DriveAccountBindingStore {
    private val preferences =
        context.applicationContext.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)

    override fun selectedIdentity(): DriveAccountIdentity? {
        val id = preferences.getString(KEY_ID, null)?.takeIf(String::isNotBlank)
        val email = preferences.getString(KEY_EMAIL, null)?.takeIf(String::isNotBlank)

        return if (id == null && email == null) {
            null
        } else {
            DriveAccountIdentity(id = id, email = email)
        }
    }

    override fun bindIdentity(identity: DriveAccountIdentity) {
        preferences
            .edit()
            .putString(KEY_ID, identity.id)
            .putString(KEY_EMAIL, identity.normalizedEmail)
            .apply()
    }

    companion object {
        private const val PREFERENCES_NAME = "cove_cloud_backup_drive_account"
        private const val KEY_ID = "google_account_id"
        private const val KEY_EMAIL = "google_account_email"
    }
}

internal fun verifyDriveAccountBinding(
    store: DriveAccountBindingStore,
    identity: DriveAccountIdentity?,
) {
    val actual = identity ?: throw DriveAccountBindingException.MissingIdentity()
    val selected = store.selectedIdentity()
    if (selected == null) {
        store.bindIdentity(actual)
        return
    }

    if (!selected.matches(actual)) {
        throw DriveAccountBindingException.Mismatch()
    }
}

internal interface DriveAuthorization {
    suspend fun accessToken(interactive: Boolean): DriveAccessToken

    suspend fun clearToken(token: String)
}

internal class CachingDriveAuthorization(
    private val delegate: DriveAuthorization,
    private val elapsedRealtime: () -> Long = ::monotonicTimeMs,
    private val cacheWindowMs: Long = ACCESS_TOKEN_CACHE_IDLE_MS,
) : DriveAuthorization {
    private val tokenMutex = Mutex()
    private var cachedAccessToken: CachedAccessToken? = null

    init {
        require(cacheWindowMs > 0) { "cacheWindowMs must be positive" }
    }

    override suspend fun accessToken(interactive: Boolean): DriveAccessToken =
        tokenMutex.withLock {
            val now = elapsedRealtime()
            cachedAccessToken?.let { cached ->
                if (cached.expiresAtMs > now) {
                    cachedAccessToken = cached.copy(expiresAtMs = now + cacheWindowMs)
                    return@withLock cached.token
                }

                cachedAccessToken = null
            }

            val token = delegate.accessToken(interactive)
            cachedAccessToken = CachedAccessToken(
                token = token,
                expiresAtMs = elapsedRealtime() + cacheWindowMs,
            )
            token
        }

    override suspend fun clearToken(token: String) {
        tokenMutex.withLock {
            if (cachedAccessToken?.token?.token == token) {
                cachedAccessToken = null
            }

            delegate.clearToken(token)
        }
    }

    private data class CachedAccessToken(
        val token: DriveAccessToken,
        val expiresAtMs: Long,
    )

    companion object {
        private const val ACCESS_TOKEN_CACHE_IDLE_MS = 2 * 60 * 1000L
    }
}

internal class DriveAuthorizationHelper(
    context: Context,
    private val selectedAccount: () -> DriveAccountIdentity? = { null },
) : DriveAuthorization {
    private val appContext = context.applicationContext
    private val client by lazy { Identity.getAuthorizationClient(appContext) }
    private val requestedScopes = listOf(Scope(DRIVE_APP_DATA_SCOPE))

    override suspend fun accessToken(interactive: Boolean): DriveAccessToken {
        val requestBuilder =
            AuthorizationRequest
                .builder()
                .setRequestedScopes(requestedScopes)

        selectedAccount()?.androidAccount()?.let(requestBuilder::setAccount)

        val authorizationResult = client
            .authorize(requestBuilder.build())
            .await()

        val resolved = resolveIfNeeded(authorizationResult, interactive)
        val token = resolved.accessToken?.takeIf { it.isNotBlank() }
            ?: throw IllegalStateException("drive authorization succeeded but returned a blank access token")
        val identity = DriveAccountIdentity.fromAuthorizationResult(resolved)
            ?: throw AuthorizationRequiredException("google drive account identity is unavailable")

        return DriveAccessToken(token = token, account = identity)
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

        val resultIntent = activityResult.data
        if (resultIntent == null) {
            if (activityResult.resultCode != Activity.RESULT_OK) {
                throw AuthorizationRequiredException("google drive authorization was cancelled")
            }

            throw IllegalStateException("google drive authorization result is missing intent data")
        }

        return try {
            client.getAuthorizationResultFromIntent(resultIntent)
        } catch (error: ApiException) {
            if (error.statusCode == CommonStatusCodes.CANCELED) {
                throw AuthorizationRequiredException("google drive authorization was cancelled", error)
            }

            throw error
        } catch (error: RuntimeException) {
            throw IllegalStateException("google drive authorization result could not be parsed", error)
        }
    }

    companion object {
        internal const val DRIVE_APP_DATA_SCOPE =
            "https://www.googleapis.com/auth/drive.appdata"
    }
}
