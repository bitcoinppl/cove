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

internal class DriveAccountIdentity(
    googleAccountId: String? = null,
    drivePermissionId: String? = null,
    email: String? = null,
) {
    val googleAccountId: String? = googleAccountId?.trim()?.takeIf(String::isNotEmpty)
    val drivePermissionId: String? = drivePermissionId?.trim()?.takeIf(String::isNotEmpty)
    val email: String? = email?.trim()?.lowercase()?.takeIf(String::isNotEmpty)

    init {
        require(this.googleAccountId != null || this.drivePermissionId != null || this.email != null) {
            "drive account identity requires google account id, drive permission id, or email"
        }
    }

    val isComplete: Boolean
        get() = (googleAccountId != null || drivePermissionId != null) && email != null

    /**
     * Fills missing fields without replacing values from the original authorization result
     */
    fun withMissingFieldsFrom(other: DriveAccountIdentity?): DriveAccountIdentity =
        if (other == null) {
            this
        } else {
            DriveAccountIdentity(
                googleAccountId = googleAccountId ?: other.googleAccountId,
                drivePermissionId = drivePermissionId ?: other.drivePermissionId,
                email = email ?: other.email,
            )
        }

    fun matches(other: DriveAccountIdentity): Boolean {
        if (googleAccountId != null && other.googleAccountId != null) {
            return googleAccountId == other.googleAccountId
        }

        if (drivePermissionId != null && other.drivePermissionId != null) {
            return drivePermissionId == other.drivePermissionId
        }

        return email != null && email == other.email
    }

    fun androidAccount(): Account? =
        email?.let { Account(it, GOOGLE_ACCOUNT_TYPE) }

    /**
     * Merges a matching identity, preferring refreshed email data from the verified identity
     */
    fun verifiedMerge(other: DriveAccountIdentity): DriveAccountIdentity {
        val googleAccountIdMatched = googleAccountId != null && googleAccountId == other.googleAccountId
        val drivePermissionIdMatched = drivePermissionId != null && drivePermissionId == other.drivePermissionId
        val mergedEmail =
            if (googleAccountIdMatched || drivePermissionIdMatched) {
                other.email ?: email
            } else {
                email ?: other.email
            }

        return DriveAccountIdentity(
            googleAccountId = googleAccountId ?: other.googleAccountId,
            drivePermissionId = drivePermissionId ?: other.drivePermissionId,
            email = mergedEmail,
        )
    }

    override fun equals(other: Any?): Boolean {
        if (this === other) {
            return true
        }

        if (other !is DriveAccountIdentity) {
            return false
        }

        return googleAccountId == other.googleAccountId &&
            drivePermissionId == other.drivePermissionId &&
            email == other.email
    }

    override fun hashCode(): Int {
        var result = googleAccountId?.hashCode() ?: 0
        result = 31 * result + (drivePermissionId?.hashCode() ?: 0)
        result = 31 * result + (email?.hashCode() ?: 0)
        return result
    }

    override fun toString(): String =
        "DriveAccountIdentity(googleAccountId=$googleAccountId, drivePermissionId=$drivePermissionId, email=$email)"

    companion object {
        private const val GOOGLE_ACCOUNT_TYPE = "com.google"

        fun fromAuthorizationResult(result: AuthorizationResult): DriveAccountIdentity? {
            val account = result.toGoogleSignInAccount()
            val id = account?.id?.takeIf(String::isNotBlank)
            val email = account?.email?.takeIf(String::isNotBlank)

            return if (id == null && email == null) {
                null
            } else {
                DriveAccountIdentity(googleAccountId = id, email = email)
            }
        }
    }
}

internal data class DriveAccessToken(
    val token: String,
    val account: DriveAccountIdentity?,
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

    fun clearIdentity()

    fun pendingTransitionId(): ULong?

    fun isIdentityStaged(transitionId: ULong): Boolean

    fun stageIdentity(transitionId: ULong, identity: DriveAccountIdentity): Boolean

    fun commitStagedIdentity(transitionId: ULong): Boolean

    fun finalizeCommittedIdentity(transitionId: ULong): Boolean

    fun rollbackStagedIdentity(transitionId: ULong): Boolean
}

internal class SharedPreferencesDriveAccountBindingStore(
    context: Context,
) : DriveAccountBindingStore {
    internal val appContext: Context = context.applicationContext
    private val preferences =
        appContext.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)

    override fun selectedIdentity(): DriveAccountIdentity? {
        if (storedPendingTransitionId() != null) {
            pendingIdentity()?.let { return it }
        }

        return identity(KEY_ID, KEY_PERMISSION_ID, KEY_EMAIL)
    }

    private fun pendingIdentity(): DriveAccountIdentity? =
        identity(KEY_PENDING_ID, KEY_PENDING_PERMISSION_ID, KEY_PENDING_EMAIL)

    private fun identity(
        idKey: String,
        permissionIdKey: String,
        emailKey: String,
    ): DriveAccountIdentity? {
        val googleAccountId = preferences.getString(idKey, null)?.takeIf(String::isNotBlank)
        val drivePermissionId = preferences.getString(permissionIdKey, null)?.takeIf(String::isNotBlank)
        val email = preferences.getString(emailKey, null)?.takeIf(String::isNotBlank)

        return if (googleAccountId == null && drivePermissionId == null && email == null) {
            null
        } else {
            DriveAccountIdentity(
                googleAccountId = googleAccountId,
                drivePermissionId = drivePermissionId,
                email = email,
            )
        }
    }

    override fun bindIdentity(identity: DriveAccountIdentity) {
        if (storedPendingTransitionId() != null) {
            preferences
                .edit()
                .putString(KEY_PENDING_ID, identity.googleAccountId)
                .putString(KEY_PENDING_PERMISSION_ID, identity.drivePermissionId)
                .putString(KEY_PENDING_EMAIL, identity.email)
                .apply()
            return
        }

        preferences
            .edit()
            .putString(KEY_ID, identity.googleAccountId)
            .putString(KEY_PERMISSION_ID, identity.drivePermissionId)
            .putString(KEY_EMAIL, identity.email)
            .apply()
    }

    override fun pendingTransitionId(): ULong? = storedPendingTransitionId()

    override fun isIdentityStaged(transitionId: ULong): Boolean =
        storedPendingTransitionId() == transitionId && pendingIdentity() != null

    private fun storedPendingTransitionId(): ULong? =
        if (preferences.contains(KEY_PENDING_TRANSITION_ID)) {
            preferences.getLong(KEY_PENDING_TRANSITION_ID, 0).toULong()
        } else {
            null
        }

    override fun stageIdentity(
        transitionId: ULong,
        identity: DriveAccountIdentity,
    ): Boolean {
        val pendingTransitionId = storedPendingTransitionId()
        if (pendingTransitionId != null && pendingTransitionId != transitionId) {
            return false
        }

        return preferences
            .edit()
            .putLong(KEY_PENDING_TRANSITION_ID, transitionId.toLong())
            .putString(KEY_PENDING_ID, identity.googleAccountId)
            .putString(KEY_PENDING_PERMISSION_ID, identity.drivePermissionId)
            .putString(KEY_PENDING_EMAIL, identity.email)
            .commit()
    }

    override fun commitStagedIdentity(transitionId: ULong): Boolean {
        val pendingTransitionId = storedPendingTransitionId()
        return when {
            pendingTransitionId == null -> storedCommittedTransitionId() == transitionId
            pendingTransitionId != transitionId -> false
            else ->
                pendingIdentity()?.let { identity ->
                    preferences
                        .edit()
                        .putString(KEY_ID, identity.googleAccountId)
                        .putString(KEY_PERMISSION_ID, identity.drivePermissionId)
                        .putString(KEY_EMAIL, identity.email)
                        .putLong(KEY_COMMITTED_TRANSITION_ID, transitionId.toLong())
                        .remove(KEY_PENDING_TRANSITION_ID)
                        .remove(KEY_PENDING_ID)
                        .remove(KEY_PENDING_PERMISSION_ID)
                        .remove(KEY_PENDING_EMAIL)
                        .commit()
                } ?: false
        }
    }

    override fun finalizeCommittedIdentity(transitionId: ULong): Boolean {
        val committedTransitionId = storedCommittedTransitionId() ?: return true
        if (committedTransitionId != transitionId) {
            return false
        }

        return preferences.edit().remove(KEY_COMMITTED_TRANSITION_ID).commit()
    }

    private fun storedCommittedTransitionId(): ULong? =
        if (preferences.contains(KEY_COMMITTED_TRANSITION_ID)) {
            preferences.getLong(KEY_COMMITTED_TRANSITION_ID, 0).toULong()
        } else {
            null
        }

    override fun rollbackStagedIdentity(transitionId: ULong): Boolean {
        val pendingTransitionId = storedPendingTransitionId()
        return when {
            pendingTransitionId == null -> true
            pendingTransitionId == transitionId ->
                preferences
                    .edit()
                    .remove(KEY_PENDING_TRANSITION_ID)
                    .remove(KEY_PENDING_ID)
                    .remove(KEY_PENDING_PERMISSION_ID)
                    .remove(KEY_PENDING_EMAIL)
                    .commit()
            else -> false
        }
    }

    override fun clearIdentity() {
        preferences
            .edit()
            .remove(KEY_ID)
            .remove(KEY_PERMISSION_ID)
            .remove(KEY_EMAIL)
            .remove(KEY_PENDING_TRANSITION_ID)
            .remove(KEY_PENDING_ID)
            .remove(KEY_PENDING_PERMISSION_ID)
            .remove(KEY_PENDING_EMAIL)
            .remove(KEY_COMMITTED_TRANSITION_ID)
            .apply()
    }

    companion object {
        private const val PREFERENCES_NAME = "cove_cloud_backup_drive_account"
        private const val KEY_ID = "google_account_id"
        private const val KEY_PERMISSION_ID = "google_drive_permission_id"
        private const val KEY_EMAIL = "google_account_email"
        private const val KEY_PENDING_TRANSITION_ID = "pending_transition_id"
        private const val KEY_PENDING_ID = "pending_google_account_id"
        private const val KEY_PENDING_PERMISSION_ID = "pending_google_drive_permission_id"
        private const val KEY_PENDING_EMAIL = "pending_google_account_email"
        private const val KEY_COMMITTED_TRANSITION_ID = "committed_transition_id"
    }
}

internal fun verifyDriveAccountBinding(
    store: DriveAccountBindingStore,
    identity: DriveAccountIdentity?,
    bindIfMissing: Boolean = true,
) {
    val actual = identity ?: throw DriveAccountBindingException.MissingIdentity()
    val selected = store.selectedIdentity()
    if (selected == null) {
        if (bindIfMissing) {
            store.bindIdentity(actual)
        }
        return
    }

    if (!selected.matches(actual)) {
        throw DriveAccountBindingException.Mismatch()
    }

    val enriched = selected.verifiedMerge(actual)
    if (bindIfMissing && enriched != selected) {
        store.bindIdentity(enriched)
    }
}

internal fun clearCloudBackupDriveAccountBinding(context: Context) {
    SharedPreferencesDriveAccountBindingStore(context.applicationContext).clearIdentity()
}

internal interface DriveAuthorization {
    suspend fun accessToken(interactive: Boolean): DriveAccessToken

    suspend fun selectAccount(): DriveAccessToken = accessToken(interactive = true)

    suspend fun updateCachedToken(accessToken: DriveAccessToken) = Unit

    /**
     * Invalidates local token state before starting failable remote clear work
     */
    suspend fun clearToken(token: String)
}

internal class CachingDriveAuthorization(
    private val delegate: DriveAuthorization,
    private val elapsedRealtime: () -> Long = ::monotonicTimeMs,
    private val cacheWindowMs: Long = ACCESS_TOKEN_CACHE_IDLE_MS,
    private val cacheKey: () -> Any? = { Unit },
) : DriveAuthorization {
    private val tokenMutex = Mutex()
    private var cachedAccessToken: CachedAccessToken? = null

    init {
        require(cacheWindowMs > 0) { "cacheWindowMs must be positive" }
    }

    override suspend fun accessToken(interactive: Boolean): DriveAccessToken =
        tokenMutex.withLock {
            val now = elapsedRealtime()
            val currentCacheKey = cacheKey()
            if (currentCacheKey == null) {
                cachedAccessToken = null
                return@withLock delegate.accessToken(interactive)
            }

            cachedAccessToken?.let { cached ->
                if (cached.cacheKey == currentCacheKey && cached.expiresAtMs > now) {
                    cachedAccessToken = cached.copy(expiresAtMs = now + cacheWindowMs)
                    return@withLock cached.token
                }

                cachedAccessToken = null
            }

            val token = delegate.accessToken(interactive)
            cachedAccessToken = CachedAccessToken(
                token = token,
                expiresAtMs = elapsedRealtime() + cacheWindowMs,
                cacheKey = currentCacheKey,
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

    override suspend fun selectAccount(): DriveAccessToken =
        tokenMutex.withLock {
            cachedAccessToken = null
            delegate.selectAccount()
        }

    override suspend fun updateCachedToken(accessToken: DriveAccessToken) {
        tokenMutex.withLock {
            val cached = cachedAccessToken ?: return@withLock
            val currentCacheKey = cacheKey()
            if (
                currentCacheKey == null ||
                    cached.cacheKey != currentCacheKey ||
                    cached.expiresAtMs <= elapsedRealtime() ||
                    cached.token.token != accessToken.token
            ) {
                cachedAccessToken = null
                return@withLock
            }

            cachedAccessToken = cached.copy(token = accessToken)
        }
    }

    private data class CachedAccessToken(
        val token: DriveAccessToken,
        val expiresAtMs: Long,
        val cacheKey: Any?,
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

    override suspend fun accessToken(interactive: Boolean): DriveAccessToken =
        authorize(
            account = selectedAccount()?.androidAccount(),
            prompt = AuthorizationRequest.Prompt.NOT_SET,
            interactive = interactive,
        )

    // explicit switching must omit the saved account so a stale credential cannot suppress the chooser
    override suspend fun selectAccount(): DriveAccessToken =
        authorize(
            account = null,
            prompt = AuthorizationRequest.Prompt.SELECT_ACCOUNT,
            interactive = true,
        )

    private suspend fun authorize(
        account: Account?,
        prompt: Int,
        interactive: Boolean,
    ): DriveAccessToken {
        val requestBuilder =
            AuthorizationRequest
                .builder()
                .setRequestedScopes(requestedScopes)

        account?.let(requestBuilder::setAccount)
        if (prompt != AuthorizationRequest.Prompt.NOT_SET) {
            requestBuilder.setPrompt(prompt)
        }

        val authorizationResult = client
            .authorize(requestBuilder.build())
            .await()

        val resolved = resolveIfNeeded(authorizationResult, interactive)
        val token = resolved.accessToken?.takeIf { it.isNotBlank() }
            ?: throw IllegalStateException("drive authorization succeeded but returned a blank access token")
        val identity = DriveAccountIdentity.fromAuthorizationResult(resolved)

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
