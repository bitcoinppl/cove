package org.bitcoinppl.cove.cloudbackup

import java.net.HttpURLConnection
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CompletableDeferred
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue

internal val testDrivePathNames = DrivePathNames(
    namespacesRootFolderName = "cspp-namespaces",
    masterKeyFolderName = "master-key",
    walletsFolderName = "wallets",
    walletFilePrefix = "wallet-",
)

internal class MockDriveServer : AutoCloseable {
    private val responses = ArrayDeque<TestDriveResponse>()
    private val server = MockWebServer()

    init {
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse {
                val response =
                    synchronized(responses) {
                        responses.removeFirstOrNull()
                            ?: TestDriveResponse(HttpURLConnection.HTTP_INTERNAL_ERROR, "unexpected request")
                    }

                return MockResponse()
                    .setResponseCode(response.statusCode)
                    .setHeader("Content-Type", "application/json")
                    .setBody(response.body)
            }
        }
        server.start()
    }

    val baseUrl: String = server.url("/").toString().removeSuffix("/")

    fun enqueue(statusCode: Int, body: String) {
        synchronized(responses) {
            responses.add(TestDriveResponse(statusCode, body))
        }
    }

    fun requests(count: Int): List<RecordedRequest> {
        val requests =
            List(count) { index ->
                server.takeRequest(1, TimeUnit.SECONDS)
                    ?: error("missing drive request index=$index")
            }
        val extra = server.takeRequest(100, TimeUnit.MILLISECONDS)
        check(extra == null) { "unexpected drive request ${extra?.requestLine}" }

        return requests
    }

    override fun close() {
        server.shutdown()
    }
}

internal data class TestDriveResponse(
    val statusCode: Int,
    val body: String,
)

internal class FailingDriveAuthorization(
    private val error: Throwable,
) : DriveAuthorization {
    override suspend fun accessToken(interactive: Boolean): DriveAccessToken {
        throw error
    }

    override suspend fun clearToken(token: String) = Unit
}

internal class SequentialDriveAuthorization(
    tokens: List<String>,
) : DriveAuthorization {
    private val tokens = ArrayDeque(tokens)
    val accessRequests = mutableListOf<Boolean>()
    val clearedTokens = mutableListOf<String>()

    override suspend fun accessToken(interactive: Boolean): DriveAccessToken {
        accessRequests.add(interactive)
        if (tokens.isEmpty()) {
            throw AssertionError("unexpected drive access token request")
        }

        return DriveAccessToken(token = tokens.removeFirst(), account = null)
    }

    override suspend fun clearToken(token: String) {
        clearedTokens.add(token)
    }
}

internal class RecordingDriveAuthorization : DriveAuthorization {
    var token = "token-1"
    var account: DriveAccountIdentity? =
        DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")
    val accessRequests = mutableListOf<Boolean>()
    val clearedTokens = mutableListOf<String>()

    override suspend fun accessToken(interactive: Boolean): DriveAccessToken {
        accessRequests.add(interactive)
        return DriveAccessToken(token = token, account = account)
    }

    override suspend fun clearToken(token: String) {
        clearedTokens.add(token)
    }
}

internal class BlockingClearDriveAuthorization : DriveAuthorization {
    var token = "token-1"
    var account = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")
    val accessRequests = mutableListOf<Boolean>()
    val clearedTokens = mutableListOf<String>()
    val clearStarted = CompletableDeferred<Unit>()
    val finishClear = CompletableDeferred<Unit>()

    override suspend fun accessToken(interactive: Boolean): DriveAccessToken {
        accessRequests.add(interactive)
        return DriveAccessToken(token = token, account = account)
    }

    override suspend fun clearToken(token: String) {
        clearedTokens.add(token)
        clearStarted.complete(Unit)
        finishClear.await()
    }
}

internal class TestDriveAccountBindingStore(
    private var identity: DriveAccountIdentity? = null,
) : DriveAccountBindingStore {
    private var pendingTransitionId: ULong? = null
    private var pendingIdentity: DriveAccountIdentity? = null
    private var committedTransitionId: ULong? = null
    var stageSucceeds = true
    var retainFailedStage = false

    override fun selectedIdentity(): DriveAccountIdentity? = pendingIdentity ?: identity

    fun committedIdentity(): DriveAccountIdentity? = identity

    override fun bindIdentity(identity: DriveAccountIdentity) {
        if (pendingTransitionId != null) {
            pendingIdentity = identity
        } else {
            this.identity = identity
            committedTransitionId = null
        }
    }

    override fun clearIdentity() {
        identity = null
        pendingTransitionId = null
        pendingIdentity = null
        committedTransitionId = null
    }

    override fun pendingTransitionId(): ULong? = pendingTransitionId

    override fun isIdentityStaged(transitionId: ULong): Boolean =
        pendingTransitionId == transitionId && pendingIdentity != null

    override fun stageIdentity(
        transitionId: ULong,
        identity: DriveAccountIdentity,
    ): Boolean {
        if (!stageSucceeds) {
            if (retainFailedStage) {
                pendingTransitionId = transitionId
                pendingIdentity = identity
            }
            return false
        }
        if (pendingTransitionId != null && pendingTransitionId != transitionId) return false

        pendingTransitionId = transitionId
        pendingIdentity = identity
        return true
    }

    override fun commitStagedIdentity(transitionId: ULong): Boolean {
        if (pendingTransitionId == null) return committedTransitionId == transitionId
        if (pendingTransitionId != transitionId) return false
        val stagedIdentity = pendingIdentity ?: return false

        identity = stagedIdentity
        pendingTransitionId = null
        pendingIdentity = null
        committedTransitionId = transitionId
        return true
    }

    override fun rollbackStagedIdentity(transitionId: ULong): Boolean {
        if (pendingTransitionId == null) return true
        if (pendingTransitionId != transitionId) return false

        pendingTransitionId = null
        pendingIdentity = null
        return true
    }
}

internal fun driveErrorBody(reason: String): String =
    """
    {
        "error": {
            "errors": [
                { "reason": "$reason" }
            ]
        }
    }
    """.trimIndent()

internal fun disabledDriveApiBody(): String =
    """
    {
        "error": {
            "message": "Google Drive API has not been used in project 738970325901 before or it is disabled.",
            "errors": [
                { "reason": "accessNotConfigured" }
            ],
            "details": [
                { "reason": "SERVICE_DISABLED" }
            ]
        }
    }
    """.trimIndent()

internal fun cloudStorageFailureMessage(error: Throwable?): String =
    when (error) {
        null -> "listNamespaces succeeded"
        is CloudStorageException.AuthorizationRequired ->
            "listNamespaces failed with AuthorizationRequired: ${error.v1}"
        is CloudStorageException.NotAvailable ->
            "listNamespaces failed with NotAvailable: ${error.v1}"
        is CloudStorageException.NotFound ->
            "listNamespaces failed with NotFound: ${error.v1}"
        is CloudStorageException.Offline ->
            "listNamespaces failed with Offline: ${error.v1}"
        is CloudStorageException.QuotaExceeded ->
            "listNamespaces failed with QuotaExceeded"
        is CloudStorageException.UploadFailed ->
            "listNamespaces failed with UploadFailed: ${error.v1}"
        is CloudStorageException.DownloadFailed ->
            "listNamespaces failed with DownloadFailed: ${error.v1}"
        else -> "listNamespaces failed with ${error.javaClass.name}: ${error.message}"
    }

internal suspend fun captureError(block: suspend () -> Unit): Throwable? =
    runCatching { block() }.exceptionOrNull()

internal fun assertNotFoundTarget(error: Throwable?, expected: String) {
    assertTrue(error is CloudStorageException.NotFound)
    assertEquals(expected, (error as CloudStorageException.NotFound).v1)
}
