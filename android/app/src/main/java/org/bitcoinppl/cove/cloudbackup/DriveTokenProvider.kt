package org.bitcoinppl.cove.cloudbackup

import java.net.HttpURLConnection
import kotlinx.coroutines.CancellationException
import org.bitcoinppl.cove_core.device.CloudStorageException

@Suppress("InstanceOfCheckForException", "ThrowsCount", "TooGenericExceptionCaught")
internal class DriveTokenProvider(
    private val driveAuthorization: DriveAuthorization,
    private val accountBindingStore: DriveAccountBindingStore,
    private val httpClient: DriveHttpClient,
) {
    suspend fun clearToken(token: String) {
        driveAuthorization.clearToken(token)
    }

    suspend fun selectAccount(): DriveAccessToken {
        val unresolvedAccess = driveAuthorization.selectAccount()
        val access =
            try {
                unresolvedAccess.withResolvedAccountIdentity()
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                clearFailedIdentityLookupToken(unresolvedAccess.token, error)

                throw error
            }
        val identity = access.account
        if (identity != null) {
            return access
        }

        runCatching {
            driveAuthorization.clearToken(access.token)
        }.onFailure { error ->
            logDriveWarning("failed to clear unidentified drive token", error)
        }

        throw DriveAccountBindingException.MissingIdentity()
    }

    suspend fun <T> runDriveOperation(
        interactive: Boolean,
        onError: (Throwable) -> CloudStorageException,
        bindAccountOnSuccess: (T) -> Boolean = { false },
        block: suspend (token: String) -> T,
    ): T {
        val started = monotonicTimeMs()
        val firstAccess =
            try {
                verifiedDriveAccessToken(interactive)
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                logDriveWarning("failed to get google drive access token", error)
                throw onError(error)
            }

        try {
            return finishDriveOperation(
                started = started,
                access = firstAccess,
                result = block(firstAccess.token),
                bindAccountOnSuccess = bindAccountOnSuccess,
            )
        } catch (error: Throwable) {
            if (error is CancellationException) throw error
            if (error is DriveHttpException && error.statusCode == HttpURLConnection.HTTP_UNAUTHORIZED) {
                runCatching {
                    driveAuthorization.clearToken(firstAccess.token)
                }.onFailure { tokenError ->
                    logDriveWarning("failed to clear expired drive token", tokenError)
                }

                val retryAccess =
                    try {
                        verifiedDriveAccessToken(interactive)
                    } catch (retryTokenError: Throwable) {
                        if (retryTokenError is CancellationException) throw retryTokenError
                        logDriveWarning("failed to refresh google drive access token", retryTokenError)
                        throw onError(retryTokenError)
                    }

                try {
                    return finishDriveOperation(
                        started = started,
                        access = retryAccess,
                        result = block(retryAccess.token),
                        bindAccountOnSuccess = bindAccountOnSuccess,
                        retry = true,
                    )
                } catch (retryError: Throwable) {
                    if (retryError is CancellationException) throw retryError
                    clearRejectedAuthorizationToken(retryAccess.token, retryError)
                    throw onError(retryError)
                }
            }

            clearRejectedAuthorizationToken(firstAccess.token, error)
            throw onError(error)
        }
    }

    private suspend fun verifiedDriveAccessToken(interactive: Boolean): DriveAccessToken =
        verifiedDriveAccessToken(interactive, retryIdentityLookup = true)

    private suspend fun verifiedDriveAccessToken(
        interactive: Boolean,
        retryIdentityLookup: Boolean,
    ): DriveAccessToken {
        val unresolvedAccess = driveAuthorization.accessToken(interactive)
        val access =
            try {
                unresolvedAccess.withResolvedAccountIdentity()
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                val tokenWasCleared = clearFailedIdentityLookupToken(unresolvedAccess.token, error)
                if (tokenWasCleared && retryIdentityLookup) {
                    return verifiedDriveAccessToken(interactive, retryIdentityLookup = false)
                }

                throw error
            }
        if (access != unresolvedAccess) {
            driveAuthorization.updateCachedToken(access)
        }

        try {
            verifyDriveAccountBinding(accountBindingStore, access.account, bindIfMissing = false)
        } catch (error: DriveAccountBindingException) {
            runCatching {
                driveAuthorization.clearToken(access.token)
            }.onFailure { tokenError ->
                logDriveWarning("failed to clear mismatched drive token", tokenError)
            }

            throw error
        }

        return access
    }

    private suspend fun DriveAccessToken.withResolvedAccountIdentity(): DriveAccessToken {
        val selectedAccount = accountBindingStore.selectedIdentity()
        if (account?.isComplete == true && !account.missingSelectedDrivePermissionId(selectedAccount)) {
            return this
        }

        val resolvedAccount = driveAccountIdentity(token)
        val mergedAccount = account?.withMissingFieldsFrom(resolvedAccount) ?: resolvedAccount

        return if (mergedAccount == account) {
            this
        } else {
            copy(account = mergedAccount)
        }
    }

    private fun DriveAccountIdentity.missingSelectedDrivePermissionId(
        selectedAccount: DriveAccountIdentity?,
    ): Boolean =
        drivePermissionId == null && selectedAccount?.drivePermissionId != null

    private suspend fun driveAccountIdentity(token: String): DriveAccountIdentity? {
        try {
            return driveAccountIdentityFromAboutResponse(
                httpClient.driveRequest(
                    token = token,
                    method = "GET",
                    url = driveApiUrl(
                        httpClient.endpoints.aboutEndpoint,
                        listOf("fields" to "user(emailAddress,permissionId)"),
                    ),
                ).asJsonObject(),
            )
        } catch (error: Throwable) {
            if (error is CancellationException) throw error

            throw DriveAccountIdentityLookupException(error)
        }
    }

    private suspend fun clearFailedIdentityLookupToken(token: String, error: Throwable): Boolean {
        val lookupError = (error as? DriveAccountIdentityLookupException)?.cause ?: error
        val logMessage =
            when {
                lookupError is DriveHttpException &&
                    lookupError.statusCode == HttpURLConnection.HTTP_UNAUTHORIZED ->
                    "failed to clear expired drive token"
                lookupError is DriveHttpException &&
                    lookupError.statusCode == HttpURLConnection.HTTP_FORBIDDEN &&
                    lookupError.isAuthorizationRejected() ->
                    "failed to clear rejected drive token"
                else -> return false
            }

        runCatching {
            driveAuthorization.clearToken(token)
        }.onFailure { tokenError ->
            logDriveWarning(logMessage, tokenError)
        }

        return true
    }

    private suspend fun <T> finishDriveOperation(
        started: Long,
        access: DriveAccessToken,
        result: T,
        bindAccountOnSuccess: (T) -> Boolean,
        retry: Boolean = false,
    ): T {
        if (bindAccountOnSuccess(result)) {
            bindDriveAccountAfterSuccessfulOperation(access)
        }

        val retryLabel = if (retry) " retry" else ""
        logDriveDebug("drive operation$retryLabel elapsed_ms=${monotonicTimeMs() - started}")
        return result
    }

    private suspend fun bindDriveAccountAfterSuccessfulOperation(access: DriveAccessToken) {
        try {
            verifyDriveAccountBinding(accountBindingStore, access.account)
        } catch (error: DriveAccountBindingException) {
            runCatching {
                driveAuthorization.clearToken(access.token)
            }.onFailure { tokenError ->
                logDriveWarning("failed to clear mismatched drive token", tokenError)
            }

            throw error
        }
    }

    private suspend fun clearRejectedAuthorizationToken(token: String, error: Throwable) {
        if (
            error is DriveHttpException &&
                error.statusCode == HttpURLConnection.HTTP_FORBIDDEN &&
                error.isAuthorizationRejected()
        ) {
            runCatching {
                driveAuthorization.clearToken(token)
            }.onFailure { tokenError ->
                logDriveWarning("failed to clear rejected drive token", tokenError)
            }
        }
    }
}
