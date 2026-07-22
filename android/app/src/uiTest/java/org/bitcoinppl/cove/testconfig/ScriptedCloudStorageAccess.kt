package org.bitcoinppl.cove.testconfig

import android.content.Context
import android.os.Process
import kotlinx.coroutines.CompletableDeferred
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
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
    private val fixtureWalletDownloadResults =
        ConcurrentHashMap<String, ConcurrentLinkedQueue<FixtureWalletDownloadResult>>()
    private val fixtureWalletDownloadDefaults =
        ConcurrentHashMap<String, FixtureWalletDownloadResult>()

    @Volatile
    private var walletListBlock: WalletListBlock? = null

    @Volatile
    private var walletDownloadBlock: WalletDownloadBlock? = null

    @Volatile
    private var fixtureWalletRecordIds = listOf(ScriptedCloudBackupFixture.WALLET_RECORD_ID)

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
        fixtureWalletDownloadRequests.clear()
        fixtureWalletDownloadResults.clear()
        fixtureWalletDownloadDefaults.clear()
        walletListBlock = null
        walletDownloadBlock = null
        fixtureWalletRecordIds = listOf(ScriptedCloudBackupFixture.WALLET_RECORD_ID)
    }

    fun exposeAllProductionFixtureWallets() {
        check(fixtureRestoreEnabled) { "production fixture restore is not configured" }
        fixtureWalletRecordIds = ScriptedCloudBackupFixture.WALLET_RECORD_IDS
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

    fun masterDownloadCount(): Int = masterDownloads.get()

    fun walletListCount(): Int = walletLists.get()

    fun walletListNamespaces(): List<String> = walletListNamespaces.toList()

    fun walletDownloadCount(): Int = walletDownloads.get()

    fun walletDownloadCount(recordId: String): Int =
        fixtureWalletDownloadRequests.count { it == recordId }

    fun walletDownloadRecordIds(): List<String> = fixtureWalletDownloadRequests.toList()

    fun blockNextWalletList() {
        check(walletListBlock?.claimed?.get() != false) { "a wallet-list block is already pending" }
        walletListBlock = WalletListBlock()
    }

    fun awaitBlockedWalletList(timeoutMs: Long = 10_000): Boolean =
        walletListBlock?.entered?.await(timeoutMs, TimeUnit.MILLISECONDS) == true

    fun releaseBlockedWalletList() {
        walletListBlock?.release?.complete(Unit)
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
        fixtureRestoreEnabled = false
        namespaceScenario = NamespaceScenario.immediateEmpty()
        freshEnableScenario = FreshEnableScenario(blockMasterUploadReturn, storage = storage)
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
        val enableScenario = freshEnableScenario

        return if (enableScenario == null) {
            namespaceScenario.next(policy)
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

        return ScriptedCloudBackupFixture.masterWrapper.copyOf()
    }

    override suspend fun listWalletFiles(
        namespace: String,
        policy: CloudAccessPolicy,
    ): List<String> {
        walletLists.incrementAndGet()
        walletListNamespaces += namespace
        walletListBlock?.let { block ->
            if (block.claimed.compareAndSet(false, true)) {
                block.entered.countDown()
                block.release.await()
            }
        }
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

        return fixtureWalletRecordIds.map(::walletFilename)
    }

    override suspend fun listWalletFilesSnapshot(
        namespace: String,
        policy: CloudAccessPolicy,
    ): CloudStorageInventorySnapshot =
        CloudStorageInventorySnapshot(
            names = listWalletFiles(namespace, policy),
            isComplete = true,
        )

    override suspend fun isBackupUploaded(
        namespace: String,
        recordId: String,
        locations: List<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ): Boolean {
        if (fixtureRestoreEnabled) {
            return namespace == ScriptedCloudBackupFixture.NAMESPACE &&
                (recordId == MASTER_KEY_RECORD_ID || recordId in fixtureWalletRecordIds)
        }

        val scenario = freshEnableScenario ?: return false
        if (!scenario.isVisibilityReleased()) return false

        return if (recordId == MASTER_KEY_RECORD_ID) {
            scenario.masterBackup(namespace) != null
        } else {
            scenario.walletBackup(namespace, recordId) != null
        }
    }

    override suspend fun overallSyncHealth(policy: CloudAccessPolicy): CloudSyncHealth {
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
        if (fixtureRestoreEnabled) {
            check(namespace == ScriptedCloudBackupFixture.NAMESPACE) {
                "unexpected fixture namespace $namespace"
            }

            return
        }

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
        if (fixtureRestoreEnabled) {
            check(namespace == ScriptedCloudBackupFixture.NAMESPACE) {
                "unexpected fixture namespace $namespace"
            }
            check(recordId in ScriptedCloudBackupFixture.WALLET_RECORD_IDS) {
                "unexpected fixture wallet record id $recordId"
            }

            return
        }

        val scenario = freshEnableScenario ?: unsupported()
        scenario.storeWalletBackup(namespace, recordId, data)
        scenario.allProviderWritesAccepted.countDown()
    }

    override suspend fun downloadWalletBackup(
        namespace: String,
        recordId: String,
        locations: List<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ): ByteArray {
        walletDownloads.incrementAndGet()
        val scenario = freshEnableScenario
        if (scenario == null) {
            if (fixtureRestoreEnabled && namespace == ScriptedCloudBackupFixture.NAMESPACE) {
                val wrapper = ScriptedCloudBackupFixture.walletWrapper(recordId) ?: unsupported()
                fixtureWalletDownloadRequests += recordId
                walletDownloadBlock?.let { block ->
                    if (block.shouldClaim(recordId)) {
                        block.entered.countDown()
                        block.release.await()
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
        val scenario = freshEnableScenario ?: unsupported()
        scenario.deleteWalletBackup(namespace, recordId)
    }

    override suspend fun deleteNamespace(
        namespace: String,
        policy: CloudAccessPolicy,
    ) {
        val scenario = freshEnableScenario ?: unsupported()
        scenario.deleteNamespace(namespace)
    }

    private fun unsupported(): Nothing =
        throw CloudStorageException.NotAvailable("operation is not scripted for UI tests")

    private fun walletFilename(recordId: String): String = "wallet-$recordId.json"

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
    ) {
        val claimed = AtomicBoolean(false)
        val entered = CountDownLatch(1)
        val release = CompletableDeferred<Unit>()
        private val remainingRequestsToSkip = AtomicInteger(matchingRequestsToSkip)

        fun shouldClaim(requestedRecordId: String): Boolean {
            if (recordId != requestedRecordId || claimed.get()) return false
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
