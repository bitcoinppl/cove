package org.bitcoinppl.cove.testconfig

import android.content.Context
import android.os.Process
import kotlinx.coroutines.CompletableDeferred
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
import org.bitcoinppl.cove_core.device.CloudBackupUploadStatus
import org.bitcoinppl.cove_core.device.CloudStorageAccess
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.bitcoinppl.cove_core.device.CloudStorageInventorySnapshot
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.bitcoinppl.cove_core.device.RemoteBackupLocation
import java.io.File
import java.io.FileOutputStream
import java.util.Base64
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

object ScriptedCloudStorageAccess : CloudStorageAccess {
    private const val MASTER_KEY_RECORD_ID = "cspp-master-key-v1"
    private const val NAMESPACE_REQUEST_POLL_INTERVAL_MS = 10L
    private const val OFFLINE_SYNC_HEALTH_MESSAGE =
        "Cove couldn't reach Google Drive. Reconnect to the internet, then try again."

    // wallet backups the device wrote after a fixture restore, served back the way Google Drive
    // would, so checking an upload finds what was written rather than the original fixture
    private val uploadedFixtureWalletBackups = ConcurrentHashMap<String, ByteArray>()

    // uploads of wallets added on the device after a fixture restore, which are not the fixture's own
    private val acceptedNewWalletUploads = AtomicInteger(0)
    private val rejectedNewWalletUploads = AtomicInteger(0)
    private val rejectNextNewWalletUpload = AtomicBoolean(false)

    @Volatile
    private var namespaceScenario = NamespaceScenario.immediateEmpty()

    @Volatile
    private var freshEnableScenario: FreshEnableScenario? = null

    @Volatile
    private var fixtureRestoreEnabled = false

    @Volatile
    private var persistentRoot: File? = null

    private val masterDownloads = AtomicInteger(0)
    private val walletLists = AtomicInteger(0)
    private val walletListNamespaces = CopyOnWriteArrayList<String>()
    private val walletDownloads = AtomicInteger(0)
    private val fixtureWalletDownloadRequests = CopyOnWriteArrayList<String>()

    // restoring a cloud-only wallet reads it with consent allowed, while startup's automatic reads
    // of a cloud-only record are silent, so these count restore attempts, finished or not
    private val consentWalletDownloadRequests = CopyOnWriteArrayList<String>()

    // fixture wallets the app deleted, in call order, so tests can assert exactly which backup went
    private val deletedFixtureWalletRecordIds = CopyOnWriteArrayList<String>()

    // deletes whose locations named another record, which production providers would have removed instead
    private val deleteLocationErrors = CopyOnWriteArrayList<String>()

    private val fixtureWalletDownloadResults =
        ConcurrentHashMap<String, ConcurrentLinkedQueue<FixtureWalletDownloadResult>>()
    private val fixtureWalletDownloadDefaults =
        ConcurrentHashMap<String, FixtureWalletDownloadResult>()

    @Volatile
    private var walletListBlock: WalletListBlock? = null

    // unlike walletListBlock, parks every plain listing until released, so concurrent readers such
    // as discovery, sync health and passkey verification can't confirm an inventory around a hold
    @Volatile
    private var walletListGate: WalletListBlock? = null

    // held apart from walletListBlock because a detail refresh also starts a sync-health check,
    // whose plain listing could otherwise take the hold meant for the refresh's snapshot
    @Volatile
    private var walletSnapshotListBlock: WalletListBlock? = null

    @Volatile
    private var walletDownloadBlock: WalletDownloadBlock? = null

    // holds keyed by record id, apart from walletDownloadBlock because several can be pending at
    // once, so a test can hold every wallet of a batch and see which one the batch reads first
    private val walletDownloadHolds = ConcurrentHashMap<String, WalletDownloadBlock>()

    @Volatile
    private var fixtureWalletRecordIds = listOf(ScriptedCloudBackupFixture.WALLET_RECORD_ID)

    private val masterUploads = AtomicInteger(0)
    private val walletUploads = AtomicInteger(0)

    // fresh-enable reads and deletes a test checks happen, or wait, without holding them
    private val uploadStatusChecks = ConcurrentHashMap<String, AtomicInteger>()
    private val namespaceDeletes = AtomicInteger(0)
    private val uploadFailures = CopyOnWriteArrayList<UploadFailure>()
    private val uploads = CopyOnWriteArrayList<Upload>()

    @Volatile
    private var walletUploadBlock: AttemptHold? = null

    private val namespaceDeleteFailures = CopyOnWriteArrayList<NamespaceDeleteFailure>()

    @Volatile
    private var namespaceDeleteHold: AttemptHold? = null

    private val walletDeletes = AtomicInteger(0)

    @Volatile
    private var walletDeleteHold: AttemptHold? = null

    // the fixture's namespace is built in, so deleting it leaves this mark until a master key
    // upload recreates it, as the iOS host's deleted-namespace marker does
    private val fixtureNamespaceDeleted = AtomicBoolean(false)

    enum class NamespaceResult {
        EMPTY,
        BACKUP_FOUND,
        OFFLINE,
        UNAVAILABLE,
    }

    enum class FixtureWalletDownloadResult {
        VALID,
        CORRUPT,
        UNAVAILABLE,
    }

    enum class UploadKind {
        MASTER,
        WALLET,
    }

    enum class UploadError {
        UPLOAD_FAILED,
        OFFLINE,
        NOT_AVAILABLE,
    }

    private data class UploadFailure(
        val kind: UploadKind,
        val attempt: Int,
        val error: UploadError,
    )

    /** How a scripted namespace delete fails */
    enum class NamespaceDeleteError {
        OFFLINE,
        NOT_AVAILABLE,
        AUTHORIZATION_REQUIRED,
        ;

        fun exception(): CloudStorageException =
            when (this) {
                OFFLINE -> CloudStorageException.Offline("scripted provider is offline")
                NOT_AVAILABLE -> CloudStorageException.NotAvailable("scripted namespace delete failure")
                AUTHORIZATION_REQUIRED -> CloudStorageException.AuthorizationRequired("scripted authorization failure")
            }
    }

    private data class NamespaceDeleteFailure(
        val attempt: Int,
        val error: NamespaceDeleteError,
    )

    /** One upload the app asked the provider to store, with [recordId] null for the master key */
    data class Upload(
        val kind: UploadKind,
        val namespace: String,
        val recordId: String?,
        val accepted: Boolean,
    )

    /** Parks the call numbered [attempt] of one kind until the test releases it */
    private class AttemptHold(
        val attempt: Int,
    ) {
        val entered = CountDownLatch(1)
        val release = CompletableDeferred<Unit>()

        suspend fun enterIfClaimedBy(call: Int) {
            if (call != attempt) return

            entered.countDown()
            release.await()
        }
    }

    fun attach(context: Context): CloudStorageAccess {
        persistentRoot = context.filesDir.resolve(".ui-test-cloud-provider")

        return this
    }

    fun configureDelayedBackupFound() {
        freshEnableScenario = null
        fixtureRestoreEnabled = false
        namespaceScenario = NamespaceScenario.delayedBackupFound()
    }

    fun configureNamespaceResults(
        vararg results: NamespaceResult,
        blockedRequest: Int? = null,
    ) {
        require(results.isNotEmpty()) { "at least one namespace result is required" }
        require(blockedRequest == null || blockedRequest > 0) {
            "blocked namespace request must be one-based"
        }

        freshEnableScenario = null
        fixtureRestoreEnabled = false
        namespaceScenario = NamespaceScenario.sequence(results.toList(), blockedRequest)
    }

    fun configureProductionFixtureRestore() {
        freshEnableScenario = null
        fixtureRestoreEnabled = true
        namespaceScenario = NamespaceScenario.sequence(listOf(NamespaceResult.BACKUP_FOUND), null)
        masterDownloads.set(0)
        walletLists.set(0)
        walletListNamespaces.clear()
        walletDownloads.set(0)
        uploadedFixtureWalletBackups.clear()
        acceptedNewWalletUploads.set(0)
        rejectedNewWalletUploads.set(0)
        rejectNextNewWalletUpload.set(false)
        fixtureWalletDownloadRequests.clear()
        consentWalletDownloadRequests.clear()
        deletedFixtureWalletRecordIds.clear()
        deleteLocationErrors.clear()
        fixtureWalletDownloadResults.clear()
        fixtureWalletDownloadDefaults.clear()
        walletListBlock = null
        walletListGate?.release?.complete(Unit)
        walletListGate = null
        walletSnapshotListBlock = null
        walletDownloadBlock = null
        releaseAllWalletDownloadHolds()
        walletDownloadHolds.clear()
        fixtureWalletRecordIds = listOf(ScriptedCloudBackupFixture.WALLET_RECORD_ID)
        fixtureNamespaceDeleted.set(false)
        resetUploads()
    }

    /** Cloud holds only the private-key fixture wallet, so restore installs an xprv wallet */
    fun configurePrivateKeyWalletRestore() {
        configureProductionFixtureRestore()
        fixtureWalletRecordIds = listOf(ScriptedCloudBackupFixture.XPRV_WALLET_RECORD_ID)
    }

    fun exposeAllProductionFixtureWallets() {
        check(fixtureRestoreEnabled) { "production fixture restore is not configured" }
        fixtureWalletRecordIds = ScriptedCloudBackupFixture.WALLET_RECORD_IDS
    }

    /**
     * Lists [recordIds] as the fixture's own records from now on, which may name any record the
     * fixture holds; backups Cove uploads during the test are listed beside them as before
     */
    fun exposeProductionFixtureWallets(recordIds: List<String>) {
        check(fixtureRestoreEnabled) { "production fixture restore is not configured" }
        require(recordIds.all { ScriptedCloudBackupFixture.walletWrapper(it) != null }) {
            "unknown fixture wallet record id in $recordIds"
        }

        fixtureWalletRecordIds = recordIds.distinct()
    }

    fun configureFixtureWalletDownloads(
        recordId: String,
        vararg results: FixtureWalletDownloadResult,
    ) {
        require(recordId in ScriptedCloudBackupFixture.WALLET_RECORD_IDS) {
            "unknown fixture wallet record id"
        }
        require(results.isNotEmpty()) { "at least one wallet download result is required" }
        fixtureWalletDownloadResults[recordId] = ConcurrentLinkedQueue(results.toList())
    }

    fun configureFixtureWalletDownloadDefault(
        recordId: String,
        result: FixtureWalletDownloadResult,
    ) {
        require(recordId in ScriptedCloudBackupFixture.WALLET_RECORD_IDS) {
            "unknown fixture wallet record id"
        }

        fixtureWalletDownloadDefaults[recordId] = result
    }

    /**
     * Rejects the next upload of a wallet that is not one of the fixture's own as over quota
     *
     * Quota errors are not retried on their own, so the wallet stays unsynced until someone asks
     * Cove to sync it again
     */
    fun rejectNextNewWalletUploadForQuota() {
        check(fixtureRestoreEnabled) { "production fixture restore is not configured" }
        rejectNextNewWalletUpload.set(true)
    }

    /** Uploads of wallets added after the fixture restore that the provider stored */
    fun acceptedNewWalletUploadCount(): Int = acceptedNewWalletUploads.get()

    /** Uploads of wallets added after the fixture restore that the provider rejected */
    fun rejectedNewWalletUploadCount(): Int = rejectedNewWalletUploads.get()

    fun masterDownloadCount(): Int = masterDownloads.get()

    /**
     * Upload confirmation checks the fresh-enable cloud answered for the record checked most
     * often, visible or not, so one verifier pass over several records never reads as polling twice
     */
    fun mostUploadStatusChecksOfOneRecord(): Int = uploadStatusChecks.values.maxOfOrNull { it.get() } ?: 0

    /** Namespace deletes either cloud was asked to make, failed or held ones included */
    fun namespaceDeleteCount(): Int = namespaceDeletes.get()

    /**
     * Fails the namespace delete numbered [attempt], counted from one since the scenario was
     * configured, after any hold on it is released; the cloud keeps the backup
     */
    fun failNamespaceDelete(
        attempt: Int,
        error: NamespaceDeleteError = NamespaceDeleteError.NOT_AVAILABLE,
    ) {
        require(attempt > 0) { "namespace delete attempts are one-based" }
        namespaceDeleteFailures += NamespaceDeleteFailure(attempt, error)
    }

    /** Fails every namespace delete numbered in [attempts], as [failNamespaceDelete] does for one */
    fun failNamespaceDeletes(
        attempts: IntRange,
        error: NamespaceDeleteError = NamespaceDeleteError.NOT_AVAILABLE,
    ) {
        attempts.forEach { failNamespaceDelete(it, error) }
    }

    /** Lets every later namespace delete through, such as the one a test's Try Again makes */
    fun stopFailingNamespaceDeletes() {
        namespaceDeleteFailures.clear()
    }

    /**
     * Holds the next namespace delete before the cloud deletes anything, until
     * [releaseHeldNamespaceDelete], so a test can check what the app allows while it is in flight
     */
    fun holdNextNamespaceDelete() {
        namespaceDeleteHold?.release?.complete(Unit)
        namespaceDeleteHold = AttemptHold(attempt = namespaceDeletes.get() + 1)
    }

    fun awaitHeldNamespaceDelete(timeoutMs: Long = 20_000): Boolean =
        namespaceDeleteHold?.entered?.await(timeoutMs, TimeUnit.MILLISECONDS) == true

    fun releaseHeldNamespaceDelete() {
        namespaceDeleteHold?.release?.complete(Unit)
    }

    /**
     * Holds the next wallet backup delete before the cloud deletes anything, until
     * [releaseHeldWalletDelete], so a test can check what the app allows while a row delete runs
     */
    fun holdNextWalletDelete() {
        walletDeleteHold?.release?.complete(Unit)
        walletDeleteHold = AttemptHold(attempt = walletDeletes.get() + 1)
    }

    fun awaitHeldWalletDelete(timeoutMs: Long = 20_000): Boolean =
        walletDeleteHold?.entered?.await(timeoutMs, TimeUnit.MILLISECONDS) == true

    fun releaseHeldWalletDelete() {
        walletDeleteHold?.release?.complete(Unit)
    }

    fun walletListCount(): Int = walletLists.get()

    fun walletListNamespaces(): List<String> = walletListNamespaces.toList()

    fun walletDownloadCount(): Int = walletDownloads.get()

    fun walletDownloadCount(recordId: String): Int =
        fixtureWalletDownloadRequests.count { it == recordId }

    fun walletDownloadRecordIds(): List<String> = fixtureWalletDownloadRequests.toList()

    /**
     * Downloads of [recordId] made with consent allowed in this process, including reads of a
     * backup Cove uploaded during the test
     *
     * Restoring a cloud-only wallet, alone or in Restore All, reads it this way, and the automatic
     * reads of a cloud-only record are silent, so a count above zero shows a restore was attempted
     */
    fun consentWalletDownloadCount(recordId: String): Int =
        consentWalletDownloadRequests.count { it == recordId }

    /** Fixture wallet record ids the app deleted from the cloud, one entry per delete call */
    fun deletedWalletRecordIds(): List<String> = deletedFixtureWalletRecordIds.toList()

    /** Deletes refused because their locations didn't name the record being deleted */
    fun deleteLocationErrors(): List<String> = deleteLocationErrors.toList()

    /** Wallet listings that returned a result, so a test can tell a refresh's listing finished */

    fun blockNextWalletList() {
        check(walletListBlock?.claimed?.get() != false) { "a wallet-list block is already pending" }
        walletListBlock = WalletListBlock()
    }

    fun awaitBlockedWalletList(timeoutMs: Long = 10_000): Boolean =
        walletListBlock?.entered?.await(timeoutMs, TimeUnit.MILLISECONDS) == true

    fun releaseBlockedWalletList() {
        walletListBlock?.release?.complete(Unit)
    }

    /** Holds every plain wallet listing from now until [releaseWalletListGate] */
    fun closeWalletListGate() {
        check(walletListGate == null) { "the wallet-list gate is already closed" }
        walletListGate = WalletListBlock()
    }

    /** Waits until a listing reaches the closed gate */
    fun awaitWalletListAtGate(timeoutMs: Long = 10_000): Boolean =
        walletListGate?.entered?.await(timeoutMs, TimeUnit.MILLISECONDS) == true

    /** Lets every listing parked at the gate, and every later one, through */
    fun releaseWalletListGate() {
        walletListGate?.release?.complete(Unit)
        walletListGate = null
    }

    /** Holds the next inventory snapshot listing, which only a Cloud Backup detail refresh makes */
    fun blockNextWalletListSnapshot() {
        check(walletSnapshotListBlock?.claimed?.get() != false) {
            "a wallet-list snapshot block is already pending"
        }
        walletSnapshotListBlock = WalletListBlock()
    }

    fun awaitBlockedWalletListSnapshot(timeoutMs: Long = 10_000): Boolean =
        walletSnapshotListBlock?.entered?.await(timeoutMs, TimeUnit.MILLISECONDS) == true

    fun releaseBlockedWalletListSnapshot() {
        walletSnapshotListBlock?.release?.complete(Unit)
    }

    /** Ends the held snapshot listing with [error], as a provider that failed the listing would */
    fun failBlockedWalletListSnapshot(error: CloudStorageException) {
        walletSnapshotListBlock?.release?.completeExceptionally(error)
    }

    fun blockNextWalletDownload(
        recordId: String,
        matchingRequestsToSkip: Int = 0,
    ) {
        require(matchingRequestsToSkip >= 0) { "matching requests to skip must not be negative" }
        check(walletDownloadBlock?.claimed?.get() != false) {
            "a wallet-download block is already pending"
        }
        walletDownloadBlock = WalletDownloadBlock(recordId, matchingRequestsToSkip)
    }

    fun awaitBlockedWalletDownload(timeoutMs: Long = 10_000): Boolean =
        walletDownloadBlock?.entered?.await(timeoutMs, TimeUnit.MILLISECONDS) == true

    fun releaseBlockedWalletDownload() {
        walletDownloadBlock?.release?.complete(Unit)
    }

    /**
     * Holds the download of [recordId] that comes after [matchingRequestsToSkip] more downloads of
     * it, until [releaseWalletDownloadHold]
     *
     * Unlike [blockNextWalletDownload], holds on different records can be pending together. Only
     * downloads of the fixture's own backups count, not ones Cove uploaded during the test, and
     * with [policy] only downloads made under that policy, so a user-started read such as a
     * restore can be told from a background inventory refresh reading the same record
     */
    fun holdWalletDownload(
        recordId: String,
        matchingRequestsToSkip: Int = 0,
        policy: CloudAccessPolicy? = null,
    ) {
        require(matchingRequestsToSkip >= 0) { "matching requests to skip must not be negative" }
        // a hold still parking a download must be released first, or replacing it would strand
        // that download where no release reaches it
        check(walletDownloadHolds[recordId]?.release?.isCompleted != false) {
            "a download hold for $recordId is still pending or parked"
        }

        walletDownloadHolds[recordId] = WalletDownloadBlock(recordId, matchingRequestsToSkip, policy)
    }

    fun awaitWalletDownloadHold(
        recordId: String,
        timeoutMs: Long = 10_000,
    ): Boolean = walletDownloadHolds[recordId]?.entered?.await(timeoutMs, TimeUnit.MILLISECONDS) == true

    /** Records whose download is parked in a hold right now, so a test can see what runs at once */
    fun heldWalletDownloadRecordIds(): Set<String> =
        walletDownloadHolds
            .filterValues { hold -> hold.entered.count == 0L && !hold.release.isCompleted }
            .keys

    fun releaseWalletDownloadHold(recordId: String) {
        walletDownloadHolds[recordId]?.release?.complete(Unit)
    }

    /** Releases every hold, entered or not, so a failed test leaves no download parked */
    fun releaseAllWalletDownloadHolds() {
        walletDownloadHolds.values.forEach { hold -> hold.release.complete(Unit) }
    }

    fun awaitWalletListCount(
        expected: Int,
        timeoutMs: Long = 10_000,
    ): Boolean {
        val deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMs)

        while (System.nanoTime() < deadline) {
            if (walletLists.get() >= expected) return true

            Thread.sleep(NAMESPACE_REQUEST_POLL_INTERVAL_MS)
        }

        return walletLists.get() >= expected
    }

    fun configureFreshEnableWithDelayedVisibility(blockMasterUploadReturn: Boolean = false) {
        resetUploads()
        fixtureRestoreEnabled = false
        namespaceScenario = NamespaceScenario.immediateEmpty()
        freshEnableScenario = FreshEnableScenario(blockMasterUploadReturn)
    }

    fun configurePersistentFreshEnableWithDelayedVisibility(
        resetStoredState: Boolean,
        blockMasterUploadReturn: Boolean = false,
    ) {
        val storage = PersistentFreshEnableStorage(requireNotNull(persistentRoot))
        storage.prepare(resetStoredState)
        resetUploads()
        fixtureRestoreEnabled = false
        namespaceScenario = NamespaceScenario.immediateEmpty()
        freshEnableScenario = FreshEnableScenario(blockMasterUploadReturn, storage = storage)
    }

    /**
     * Fails the upload of [kind] numbered [attempt], counted from one since the scenario was
     * configured, so a test can retry a setup whose upload the provider rejected
     */
    fun failUpload(
        kind: UploadKind,
        attempt: Int,
        error: UploadError = UploadError.UPLOAD_FAILED,
    ) {
        require(attempt > 0) { "upload attempts are one-based" }
        uploadFailures += UploadFailure(kind, attempt, error)
    }

    /** Uploads since the scenario was configured, in call order */
    fun uploads(): List<Upload> = uploads.toList()

    /**
     * Holds the next wallet upload until [releaseBlockedWalletUpload], so a test can tell which
     * action made it
     */
    fun blockNextWalletUpload() {
        walletUploadBlock = AttemptHold(attempt = walletUploads.get() + 1)
    }

    fun awaitBlockedWalletUpload(timeoutMs: Long = 20_000): Boolean =
        walletUploadBlock?.entered?.await(timeoutMs, TimeUnit.MILLISECONDS) == true

    fun releaseBlockedWalletUpload() {
        walletUploadBlock?.release?.complete(Unit)
    }

    /** Namespaces the fresh-enable cloud holds a master key for */
    fun freshEnableNamespaces(): List<String> = freshEnableScenario?.masterNamespaces().orEmpty()

    fun freshEnableWalletRecordIds(namespace: String): List<String> =
        freshEnableScenario?.walletRecordIds(namespace).orEmpty()

    fun uploadCount(kind: UploadKind): Int =
        when (kind) {
            UploadKind.MASTER -> masterUploads.get()
            UploadKind.WALLET -> walletUploads.get()
        }

    fun awaitUploadCount(
        kind: UploadKind,
        expected: Int,
        timeoutMs: Long = 20_000,
    ): Boolean {
        val deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMs)

        while (System.nanoTime() < deadline) {
            if (uploadCount(kind) >= expected) return true

            Thread.sleep(NAMESPACE_REQUEST_POLL_INTERVAL_MS)
        }

        return uploadCount(kind) >= expected
    }

    fun awaitNamespaceRequest(timeoutMs: Long = 10_000): Boolean =
        awaitNamespaceRequestCount(expected = 1, timeoutMs = timeoutMs)

    fun awaitNamespaceRequestCount(
        expected: Int,
        timeoutMs: Long = 10_000,
    ): Boolean = namespaceScenario.awaitRequestCount(expected, timeoutMs)

    fun namespaceRequestCount(): Int = namespaceScenario.requestCount()

    fun namespaceRequestPolicies(): List<CloudAccessPolicy> = namespaceScenario.requestPolicies()

    fun releaseBackupFound() {
        releaseBlockedNamespaceRequest()
    }

    fun releaseBlockedNamespaceRequest() {
        namespaceScenario.releaseBlockedRequest()
    }

    fun awaitMasterWriteAccepted(timeoutMs: Long = 20_000): Boolean =
        freshEnableScenario?.masterWriteAccepted?.await(timeoutMs, TimeUnit.MILLISECONDS) == true

    fun awaitAllProviderWritesAccepted(timeoutMs: Long = 20_000): Boolean =
        freshEnableScenario?.allProviderWritesAccepted?.await(timeoutMs, TimeUnit.MILLISECONDS) == true

    fun releaseMasterUploadReturn() {
        freshEnableScenario?.masterUploadReturn?.complete(Unit)
    }

    fun releaseVisibility() {
        freshEnableScenario?.releaseVisibility()
    }

    fun isVisibilityReleased(): Boolean =
        freshEnableScenario?.isVisibilityReleased() == true

    fun awaitVisibleConfirmationRead(timeoutMs: Long = 20_000): Boolean =
        freshEnableScenario?.visibleConfirmationRead?.await(timeoutMs, TimeUnit.MILLISECONDS) == true

    fun recordProcessAndCheckRestart(expectedRestart: Boolean): Boolean =
        freshEnableScenario?.recordProcessAndCheckRestart(Process.myPid(), expectedRestart) == true

    fun recordFixtureProcessAndCheckRestart(
        resetStoredState: Boolean,
        expectedRestart: Boolean,
    ): Boolean =
        PersistentProcessMarker(requireNotNull(persistentRoot).resolve("fixture-process"))
            .recordAndCheck(
                processId = Process.myPid(),
                resetStoredState = resetStoredState,
                expectedRestart = expectedRestart,
            )

    override suspend fun listNamespaces(policy: CloudAccessPolicy): List<String> {
        ScriptedConnectivityAccess.throwIfOffline()
        val enableScenario = freshEnableScenario

        return if (enableScenario == null) {
            namespaceScenario.next(policy).filterNot(::isDeletedFixtureNamespace)
        } else if (enableScenario.isVisibilityReleased()) {
            enableScenario.masterNamespaces()
        } else {
            emptyList()
        }
    }

    override suspend fun downloadMasterKeyBackup(
        namespace: String,
        locations: List<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ): ByteArray {
        ScriptedConnectivityAccess.throwIfOffline()
        masterDownloads.incrementAndGet()
        freshEnableScenario?.let { scenario ->
            if (!scenario.isVisibilityReleased()) {
                throw CloudStorageException.NotFound("master key backup is not visible yet")
            }

            val backup =
                scenario.masterBackup(namespace)
                    ?: throw CloudStorageException.NotFound("master key backup not found")
            scenario.visibleConfirmationRead.countDown()

            return backup.copyOf()
        }

        if (isDeletedFixtureNamespace(namespace)) {
            throw CloudStorageException.NotFound("cloud backup was deleted")
        }

        return ScriptedCloudBackupFixture.masterWrapper.copyOf()
    }

    override suspend fun listWalletFiles(
        namespace: String,
        policy: CloudAccessPolicy,
    ): List<String> {
        ScriptedConnectivityAccess.throwIfOffline()
        walletLists.incrementAndGet()
        walletListNamespaces += namespace
        walletListBlock?.let { block ->
            if (block.claimed.compareAndSet(false, true)) {
                block.entered.countDown()
                block.release.await()
            }
        }

        walletListGate?.let { gate ->
            gate.entered.countDown()
            gate.release.await()
        }

        return currentWalletFilenames(namespace)
    }

    private fun currentWalletFilenames(namespace: String): List<String> {
        val enableScenario = freshEnableScenario

        if (enableScenario != null) {
            return enableScenario
                .takeIf(FreshEnableScenario::isVisibilityReleased)
                ?.walletRecordIds(namespace)
                ?.map(::walletFilename)
                ?: emptyList()
        }

        if (!fixtureRestoreEnabled) return emptyList()

        check(namespace == ScriptedCloudBackupFixture.NAMESPACE) {
            "unexpected fixture namespace $namespace"
        }
        if (isDeletedFixtureNamespace(namespace)) {
            throw CloudStorageException.NotFound("cloud backup is not available")
        }

        return visibleFixtureWalletRecordIds().map(::walletFilename)
    }

    override suspend fun listWalletFilesSnapshot(
        namespace: String,
        policy: CloudAccessPolicy,
    ): CloudStorageInventorySnapshot {
        walletSnapshotListBlock?.let { block ->
            if (block.claimed.compareAndSet(false, true)) {
                block.entered.countDown()
                block.release.await()
            }
        }

        return CloudStorageInventorySnapshot(
            names = listWalletFiles(namespace, policy),
            isComplete = true,
        )
    }

    override suspend fun isBackupUploaded(
        namespace: String,
        recordId: String,
        locations: List<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ): CloudBackupUploadStatus {
        ScriptedConnectivityAccess.throwIfOffline()

        if (fixtureRestoreEnabled) {
            return if (
                namespace == ScriptedCloudBackupFixture.NAMESPACE &&
                    !isDeletedFixtureNamespace(namespace) &&
                    (recordId == MASTER_KEY_RECORD_ID || recordId in visibleFixtureWalletRecordIds())
            ) {
                CloudBackupUploadStatus.UPLOADED
            } else {
                CloudBackupUploadStatus.NOT_FOUND
            }
        }

        val scenario = freshEnableScenario ?: return CloudBackupUploadStatus.NOT_FOUND
        uploadStatusChecks.getOrPut(recordId) { AtomicInteger(0) }.incrementAndGet()
        if (!scenario.isVisibilityReleased()) return CloudBackupUploadStatus.PENDING

        val exists = if (recordId == MASTER_KEY_RECORD_ID) {
            scenario.masterBackup(namespace) != null
        } else {
            scenario.walletBackup(namespace, recordId) != null
        }

        return if (exists) {
            CloudBackupUploadStatus.UPLOADED
        } else {
            CloudBackupUploadStatus.NOT_FOUND
        }
    }

    override suspend fun overallSyncHealth(policy: CloudAccessPolicy): CloudSyncHealth {
        // the Google Drive provider reports a failed health instead of throwing when offline
        if (!ScriptedConnectivityAccess.isConnected()) return CloudSyncHealth.Failed(OFFLINE_SYNC_HEALTH_MESSAGE)
        if (fixtureRestoreEnabled) return CloudSyncHealth.AllUploaded

        val scenario = freshEnableScenario ?: return CloudSyncHealth.NoFiles
        if (scenario.masterNamespaces().isEmpty()) return CloudSyncHealth.NoFiles

        return if (scenario.isVisibilityReleased()) {
            CloudSyncHealth.AllUploaded
        } else {
            CloudSyncHealth.Uploading
        }
    }

    override suspend fun uploadMasterKeyBackup(
        namespace: String,
        location: RemoteBackupLocation,
        data: ByteArray,
        policy: CloudAccessPolicy,
    ) {
        recordUpload(UploadKind.MASTER, masterUploads.incrementAndGet(), namespace, recordId = null)

        if (fixtureRestoreEnabled) {
            check(namespace == ScriptedCloudBackupFixture.NAMESPACE) {
                "unexpected fixture namespace $namespace"
            }
            ScriptedConnectivityAccess.throwIfOffline()
            fixtureNamespaceDeleted.set(false)

            return
        }

        ScriptedConnectivityAccess.throwIfOffline()
        val scenario = freshEnableScenario ?: unsupported()
        scenario.storeMasterBackup(namespace, data)
        scenario.masterWriteAccepted.countDown()
        scenario.allProviderWritesAccepted.countDown()

        if (scenario.blockMasterUploadReturn) {
            scenario.masterUploadReturn.await()
        }
    }

    override suspend fun uploadWalletBackup(
        namespace: String,
        recordId: String,
        location: RemoteBackupLocation,
        data: ByteArray,
        policy: CloudAccessPolicy,
    ) {
        val attempt = walletUploads.incrementAndGet()
        recordUpload(UploadKind.WALLET, attempt, namespace, recordId)
        walletUploadBlock?.enterIfClaimedBy(attempt)

        if (fixtureRestoreEnabled) {
            check(namespace == ScriptedCloudBackupFixture.NAMESPACE) {
                "unexpected fixture namespace $namespace"
            }
            ScriptedConnectivityAccess.throwIfOffline()
            val isFixtureWallet =
                recordId in fixtureWalletRecordIds || recordId in ScriptedCloudBackupFixture.WALLET_RECORD_IDS
            if (!isFixtureWallet && rejectNextNewWalletUpload.compareAndSet(true, false)) {
                rejectedNewWalletUploads.incrementAndGet()
                throw CloudStorageException.QuotaExceeded()
            }

            uploadedFixtureWalletBackups[recordId] = data.copyOf()
            if (!isFixtureWallet) acceptedNewWalletUploads.incrementAndGet()

            return
        }

        ScriptedConnectivityAccess.throwIfOffline()
        val scenario = freshEnableScenario ?: unsupported()
        scenario.storeWalletBackup(namespace, recordId, data)
        scenario.allProviderWritesAccepted.countDown()
    }

    private fun resetUploads() {
        masterUploads.set(0)
        walletUploads.set(0)
        uploadStatusChecks.clear()
        namespaceDeletes.set(0)
        uploadFailures.clear()
        uploads.clear()
        walletUploadBlock?.release?.complete(Unit)
        walletUploadBlock = null
        namespaceDeleteFailures.clear()
        namespaceDeleteHold?.release?.complete(Unit)
        namespaceDeleteHold = null
        walletDeletes.set(0)
        walletDeleteHold?.release?.complete(Unit)
        walletDeleteHold = null
    }

    /** Records the upload, and throws the scripted failure when one fails this attempt */
    private fun recordUpload(
        kind: UploadKind,
        attempt: Int,
        namespace: String,
        recordId: String?,
    ) {
        val failure = uploadFailures.firstOrNull { it.kind == kind && it.attempt == attempt }
        uploads += Upload(kind, namespace, recordId, accepted = failure == null)
        if (failure == null) return

        throw when (failure.error) {
            UploadError.UPLOAD_FAILED -> CloudStorageException.UploadFailed("scripted $kind upload failure")
            UploadError.OFFLINE -> CloudStorageException.Offline("scripted provider is offline")
            UploadError.NOT_AVAILABLE -> CloudStorageException.NotAvailable("scripted provider is unavailable")
        }
    }

    override suspend fun downloadWalletBackup(
        namespace: String,
        recordId: String,
        locations: List<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ): ByteArray {
        ScriptedConnectivityAccess.throwIfOffline()
        walletDownloads.incrementAndGet()
        if (policy == CloudAccessPolicy.CONSENT_ALLOWED) {
            consentWalletDownloadRequests += recordId
        }

        val scenario = freshEnableScenario
        if (scenario == null) {
            if (fixtureRestoreEnabled && namespace == ScriptedCloudBackupFixture.NAMESPACE) {
                if (isDeletedFixtureNamespace(namespace)) {
                    throw CloudStorageException.NotFound("cloud backup was deleted")
                }

                // a deleted backup stays gone even if Cove uploaded it earlier in the test
                if (recordId !in deletedFixtureWalletRecordIds) {
                    uploadedFixtureWalletBackups[recordId]?.let { backup -> return backup.copyOf() }
                }

                val wrapper =
                    ScriptedCloudBackupFixture.walletWrapper(recordId)
                        ?: throw CloudStorageException.NotFound("wallet backup not found")
                fixtureWalletDownloadRequests += recordId
                if (recordId in deletedFixtureWalletRecordIds) {
                    throw CloudStorageException.NotFound("scripted fixture wallet was deleted")
                }

                walletDownloadBlock?.let { block ->
                    if (block.shouldClaim(recordId)) {
                        block.entered.countDown()
                        block.release.await()
                    }
                }

                walletDownloadHolds[recordId]?.let { hold ->
                    if (hold.shouldClaim(recordId, policy)) {
                        hold.entered.countDown()
                        hold.release.await()
                    }
                }

                return when (
                    fixtureWalletDownloadResults[recordId]?.poll()
                        ?: fixtureWalletDownloadDefaults[recordId]
                        ?: FixtureWalletDownloadResult.VALID
                ) {
                    FixtureWalletDownloadResult.VALID -> {
                        wrapper.copyOf()
                    }

                    FixtureWalletDownloadResult.CORRUPT -> {
                        wrapper.corruptCiphertext()
                    }

                    FixtureWalletDownloadResult.UNAVAILABLE -> {
                        throw CloudStorageException.NotAvailable(
                            "scripted fixture wallet is unavailable",
                        )
                    }
                }
            }

            unsupported()
        }

        if (!scenario.isVisibilityReleased()) {
            throw CloudStorageException.NotFound("wallet backup is not visible yet")
        }

        val backup =
            scenario.walletBackup(namespace, recordId)
                ?: throw CloudStorageException.NotFound("wallet backup not found")
        scenario.visibleConfirmationRead.countDown()

        return backup.copyOf()
    }

    override suspend fun deleteWalletBackup(
        namespace: String,
        recordId: String,
        locations: List<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ) {
        ScriptedConnectivityAccess.throwIfOffline()
        walletDeleteHold?.enterIfClaimedBy(walletDeletes.incrementAndGet())

        val scenario = freshEnableScenario
        if (scenario != null) {
            scenario.deleteWalletBackup(namespace, recordId)
            return
        }

        if (!fixtureRestoreEnabled) unsupported()

        check(namespace == ScriptedCloudBackupFixture.NAMESPACE) {
            "unexpected fixture namespace $namespace"
        }
        check(ScriptedCloudBackupFixture.walletWrapper(recordId) != null) {
            "unexpected fixture wallet record id $recordId"
        }

        // production providers delete the files at these locations, not by record id, so they must
        // name exactly this record's current and legacy files
        val expected = listOf("wallets/${walletFilename(recordId)}", walletFilename(recordId))
        val paths = locations.map(RemoteBackupLocation::relativePath)
        if (paths.size != expected.size || paths.toSet() != expected.toSet()) {
            deleteLocationErrors += "record $recordId got locations $paths"
            throw CloudStorageException.NotAvailable("delete locations don't match record $recordId")
        }

        deletedFixtureWalletRecordIds += recordId
        uploadedFixtureWalletBackups.remove(recordId)
    }

    override suspend fun deleteNamespace(
        namespace: String,
        policy: CloudAccessPolicy,
    ) {
        ScriptedConnectivityAccess.throwIfOffline()
        val attempt = namespaceDeletes.incrementAndGet()
        namespaceDeleteHold?.enterIfClaimedBy(attempt)
        namespaceDeleteFailures.firstOrNull { it.attempt == attempt }?.let { throw it.error.exception() }

        val scenario = freshEnableScenario
        if (scenario != null) {
            scenario.deleteNamespace(namespace)
            return
        }

        if (!fixtureRestoreEnabled) unsupported()

        check(namespace == ScriptedCloudBackupFixture.NAMESPACE) {
            "unexpected fixture namespace $namespace"
        }

        // the delete takes the device's uploads with it; the built-in fixture records stay
        // hidden behind the mark
        fixtureNamespaceDeleted.set(true)
        uploadedFixtureWalletBackups.clear()
    }

    private fun isDeletedFixtureNamespace(namespace: String): Boolean =
        fixtureRestoreEnabled && namespace == ScriptedCloudBackupFixture.NAMESPACE && fixtureNamespaceDeleted.get()

    private fun unsupported(): Nothing =
        throw CloudStorageException.NotAvailable("operation is not scripted for UI tests")

    private fun walletFilename(recordId: String): String = "wallet-$recordId.json"

    // wallets Cove uploaded during the test are listed like the fixture's own;
    // a deleted fixture wallet leaves the listing, so the refresh after a delete can't bring it back;
    // after a delete the private-key fixture wallet joins it as a test-only refresh marker, so a
    // test that sees its row knows a post-delete detail refresh was applied
    private fun visibleFixtureWalletRecordIds(): List<String> {
        val refreshMarker =
            listOf(ScriptedCloudBackupFixture.XPRV_WALLET_RECORD_ID)
                .takeIf { deletedFixtureWalletRecordIds.isNotEmpty() }
                .orEmpty()

        return (fixtureWalletRecordIds + uploadedFixtureWalletBackups.keys.sorted() + refreshMarker)
            .distinct()
            .filterNot { it in deletedFixtureWalletRecordIds }
    }

    private class NamespaceScenario(
        results: List<NamespaceResult>,
        private val blockedRequest: Int?,
    ) {
        private val queuedResults = ConcurrentLinkedQueue(results)
        private val fallbackResult = results.last()
        private val requests = AtomicInteger(0)
        private val policies = CopyOnWriteArrayList<CloudAccessPolicy>()
        private val blockedRequestRelease = CompletableDeferred<Unit>()

        suspend fun next(policy: CloudAccessPolicy): List<String> {
            policies += policy
            val request = requests.incrementAndGet()

            if (request == blockedRequest) {
                blockedRequestRelease.await()
            }

            return when (queuedResults.poll() ?: fallbackResult) {
                NamespaceResult.EMPTY -> {
                    emptyList()
                }

                NamespaceResult.BACKUP_FOUND -> {
                    listOf(ScriptedCloudBackupFixture.NAMESPACE)
                }

                NamespaceResult.OFFLINE -> {
                    throw CloudStorageException.Offline("scripted provider is offline")
                }

                NamespaceResult.UNAVAILABLE -> {
                    throw CloudStorageException.NotAvailable("scripted provider is unavailable")
                }
            }
        }

        fun awaitRequestCount(
            expected: Int,
            timeoutMs: Long,
        ): Boolean {
            val deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMs)

            while (System.nanoTime() < deadline) {
                if (requests.get() >= expected) return true

                Thread.sleep(NAMESPACE_REQUEST_POLL_INTERVAL_MS)
            }

            return requests.get() >= expected
        }

        fun requestCount(): Int = requests.get()

        fun requestPolicies(): List<CloudAccessPolicy> = policies.toList()

        fun releaseBlockedRequest() {
            blockedRequestRelease.complete(Unit)
        }

        companion object {
            fun immediateEmpty() =
                sequence(listOf(NamespaceResult.EMPTY))

            fun delayedBackupFound() =
                sequence(
                    results = listOf(NamespaceResult.BACKUP_FOUND),
                    blockedRequest = 1,
                )

            fun sequence(
                results: List<NamespaceResult>,
                blockedRequest: Int? = null,
            ) = NamespaceScenario(results, blockedRequest)
        }
    }

    private data class WalletBackupKey(
        val namespace: String,
        val recordId: String,
    )

    private class WalletListBlock {
        val claimed = AtomicBoolean(false)
        val entered = CountDownLatch(1)
        val release = CompletableDeferred<Unit>()
    }

    private class WalletDownloadBlock(
        val recordId: String,
        matchingRequestsToSkip: Int,
        // null matches a download under any policy
        private val policy: CloudAccessPolicy? = null,
    ) {
        val claimed = AtomicBoolean(false)
        val entered = CountDownLatch(1)
        val release = CompletableDeferred<Unit>()
        private val remainingRequestsToSkip = AtomicInteger(matchingRequestsToSkip)

        fun shouldClaim(
            requestedRecordId: String,
            requestedPolicy: CloudAccessPolicy? = null,
        ): Boolean {
            if (recordId != requestedRecordId || claimed.get()) return false
            if (policy != null && policy != requestedPolicy) return false
            if (remainingRequestsToSkip.getAndUpdate { value -> value.coerceAtLeast(1) - 1 } > 0) {
                return false
            }

            return claimed.compareAndSet(false, true)
        }
    }

    private fun ByteArray.corruptCiphertext(): ByteArray {
        val json = decodeToString()
        val marker = "\"ciphertext\":\""
        val valueIndex =
            json.indexOf(marker).takeIf { it >= 0 }?.plus(marker.length)
                ?: error("fixture wallet ciphertext is missing")
        val replacement = if (json[valueIndex] == 'A') 'B' else 'A'

        return json.replaceRange(valueIndex, valueIndex + 1, replacement.toString()).encodeToByteArray()
    }

    private class FreshEnableScenario(
        val blockMasterUploadReturn: Boolean,
        private val storage: PersistentFreshEnableStorage? = null,
        val masterBackups: ConcurrentHashMap<String, ByteArray> = ConcurrentHashMap(),
        val walletBackups: ConcurrentHashMap<WalletBackupKey, ByteArray> = ConcurrentHashMap(),
        val masterWriteAccepted: CountDownLatch = CountDownLatch(1),
        val allProviderWritesAccepted: CountDownLatch = CountDownLatch(2),
        val masterUploadReturn: CompletableDeferred<Unit> = CompletableDeferred(),
        val visibilityReleased: AtomicBoolean = AtomicBoolean(false),
        val visibleConfirmationRead: CountDownLatch = CountDownLatch(1),
    ) {
        fun storeMasterBackup(namespace: String, data: ByteArray) {
            storage?.storeMasterBackup(namespace, data)
                ?: masterBackups.set(namespace, data.copyOf())
        }

        fun storeWalletBackup(
            namespace: String,
            recordId: String,
            data: ByteArray,
        ) {
            storage?.storeWalletBackup(namespace, recordId, data)
                ?: walletBackups.set(WalletBackupKey(namespace, recordId), data.copyOf())
        }

        fun masterBackup(namespace: String): ByteArray? =
            storage?.masterBackup(namespace) ?: masterBackups[namespace]?.copyOf()

        fun walletBackup(
            namespace: String,
            recordId: String,
        ): ByteArray? =
            storage?.walletBackup(namespace, recordId)
                ?: walletBackups[WalletBackupKey(namespace, recordId)]?.copyOf()

        fun masterNamespaces(): List<String> =
            storage?.masterNamespaces() ?: masterBackups.keys.sorted()

        fun walletRecordIds(namespace: String): List<String> =
            storage?.walletRecordIds(namespace)
                ?: walletBackups.keys
                    .filter { it.namespace == namespace }
                    .map(WalletBackupKey::recordId)
                    .sorted()

        fun deleteWalletBackup(
            namespace: String,
            recordId: String,
        ) {
            storage?.deleteWalletBackup(namespace, recordId)
                ?: walletBackups.remove(WalletBackupKey(namespace, recordId))
        }

        fun deleteNamespace(namespace: String) {
            storage?.deleteNamespace(namespace) ?: run {
                masterBackups.remove(namespace)
                walletBackups.keys.removeAll { it.namespace == namespace }
            }
        }

        fun releaseVisibility() {
            storage?.releaseVisibility() ?: visibilityReleased.set(true)
        }

        fun isVisibilityReleased(): Boolean =
            storage?.isVisibilityReleased() ?: visibilityReleased.get()

        fun recordProcessAndCheckRestart(
            processId: Int,
            expectedRestart: Boolean,
        ): Boolean =
            storage?.recordProcessAndCheckRestart(processId, expectedRestart) ?: !expectedRestart
    }

    private class PersistentFreshEnableStorage(
        private val root: File,
    ) {
        private val marker = root.resolve("scenario")
        private val process = root.resolve("process")
        private val visibility = root.resolve("visible")
        private val masters = root.resolve("masters")
        private val wallets = root.resolve("wallets")

        @Synchronized
        fun prepare(resetStoredState: Boolean) {
            if (resetStoredState) {
                check(root.deleteRecursively()) { "failed to reset scripted cloud provider state" }
                check(root.mkdirs()) { "failed to create scripted cloud provider state" }
                writeDurably(marker, byteArrayOf(1))
            }

            check(marker.isFile) { "scripted cloud provider state was not initialized" }
        }

        @Synchronized
        fun storeMasterBackup(namespace: String, data: ByteArray) {
            writeDurably(masters.resolve(encode(namespace)), data)
        }

        @Synchronized
        fun storeWalletBackup(
            namespace: String,
            recordId: String,
            data: ByteArray,
        ) {
            writeDurably(wallets.resolve(encode(namespace)).resolve(encode(recordId)), data)
        }

        @Synchronized
        fun masterBackup(namespace: String): ByteArray? =
            masters.resolve(encode(namespace)).takeIf(File::isFile)?.readBytes()

        @Synchronized
        fun walletBackup(
            namespace: String,
            recordId: String,
        ): ByteArray? =
            wallets
                .resolve(encode(namespace))
                .resolve(encode(recordId))
                .takeIf(File::isFile)
                ?.readBytes()

        @Synchronized
        fun masterNamespaces(): List<String> =
            masters
                .listFiles()
                ?.filter(File::isFile)
                ?.map { decode(it.name) }
                ?.sorted()
                ?: emptyList()

        @Synchronized
        fun walletRecordIds(namespace: String): List<String> =
            wallets
                .resolve(encode(namespace))
                .listFiles()
                ?.filter(File::isFile)
                ?.map { decode(it.name) }
                ?.sorted()
                ?: emptyList()

        @Synchronized
        fun deleteWalletBackup(
            namespace: String,
            recordId: String,
        ) {
            wallets.resolve(encode(namespace)).resolve(encode(recordId)).delete()
        }

        @Synchronized
        fun deleteNamespace(namespace: String) {
            masters.resolve(encode(namespace)).delete()
            wallets.resolve(encode(namespace)).deleteRecursively()
        }

        @Synchronized
        fun releaseVisibility() {
            writeDurably(visibility, byteArrayOf(1))
        }

        @Synchronized
        fun isVisibilityReleased(): Boolean = visibility.isFile

        @Synchronized
        fun recordProcessAndCheckRestart(
            processId: Int,
            expectedRestart: Boolean,
        ): Boolean {
            val previous = process.takeIf(File::isFile)?.readText()?.toIntOrNull()
            writeDurably(process, processId.toString().encodeToByteArray())

            return if (expectedRestart) {
                previous != null && previous != processId
            } else {
                previous == null
            }
        }

        private fun writeDurably(destination: File, data: ByteArray) {
            val parent = requireNotNull(destination.parentFile)
            check(parent.isDirectory || parent.mkdirs()) {
                "failed to create scripted cloud provider directory"
            }
            val temporary = destination.resolveSibling("${destination.name}.tmp")
            FileOutputStream(temporary).use { output ->
                output.write(data)
                output.fd.sync()
            }
            check(temporary.renameTo(destination)) {
                "failed to commit scripted cloud provider data"
            }
        }

        private fun encode(value: String): String =
            Base64.getUrlEncoder().withoutPadding().encodeToString(value.encodeToByteArray())

        private fun decode(value: String): String =
            Base64.getUrlDecoder().decode(value).decodeToString()
    }

    private class PersistentProcessMarker(
        private val marker: File,
    ) {
        @Synchronized
        fun recordAndCheck(
            processId: Int,
            resetStoredState: Boolean,
            expectedRestart: Boolean,
        ): Boolean {
            if (resetStoredState) {
                check(marker.delete() || !marker.exists()) {
                    "failed to reset scripted fixture process marker"
                }
            }

            val previous = marker.takeIf(File::isFile)?.readText()?.toIntOrNull()
            writeDurably(processId.toString().encodeToByteArray())

            return if (expectedRestart) {
                previous != null && previous != processId
            } else {
                previous == null
            }
        }

        private fun writeDurably(data: ByteArray) {
            val parent = requireNotNull(marker.parentFile)
            check(parent.isDirectory || parent.mkdirs()) {
                "failed to create scripted fixture process-marker directory"
            }
            val temporary = marker.resolveSibling("${marker.name}.tmp")
            FileOutputStream(temporary).use { output ->
                output.write(data)
                output.fd.sync()
            }
            check(temporary.renameTo(marker)) {
                "failed to commit scripted fixture process marker"
            }
        }
    }
}
