import CoveCore
import Foundation

struct ICloudMetadataRecord: Equatable, Sendable {
    let name: String
    let url: URL
    let resolvedPath: String

    func matches(name: String, parentPath: String) -> Bool {
        self.name == name
            && URL(fileURLWithPath: resolvedPath).deletingLastPathComponent().path == parentPath
    }
}

struct ICloudMetadataCandidate: Equatable, Sendable {
    let name: String
    let parentPath: String
}

struct ICloudMetadataMatch: Equatable, Sendable {
    let preferred: ICloudMetadataRecord
    let records: [ICloudMetadataRecord]

    fileprivate init?(_ records: [ICloudMetadataRecord]) {
        guard let preferred = records.first else { return nil }

        self.preferred = preferred
        self.records = records
    }
}

enum ICloudMetadataProjection {
    static func subdirectoryNames(
        in records: [ICloudMetadataRecord],
        parentPath: String
    ) -> [String] {
        let pathPrefix = parentPath + "/"
        var names = Set<String>()

        for record in records where record.resolvedPath.hasPrefix(pathPrefix) {
            let relativePath = String(record.resolvedPath.dropFirst(pathPrefix.count))
            guard let firstComponent = relativePath.split(separator: "/").first else { continue }
            names.insert(String(firstComponent))
        }
        return names.sorted()
    }

    static func fileNames(
        in records: [ICloudMetadataRecord],
        parentPath: String,
        prefix: String
    ) -> [String] {
        let pathPrefix = parentPath + "/"
        var names = Set<String>()

        for record in records where record.resolvedPath.hasPrefix(pathPrefix) {
            let relativePath = String(record.resolvedPath.dropFirst(pathPrefix.count))
            guard !relativePath.contains("/") else { continue }
            let name = URL(fileURLWithPath: relativePath).lastPathComponent
            guard name.hasPrefix(prefix) else { continue }
            names.insert(name)
        }
        return names.sorted()
    }
}

enum ICloudMetadataQueryEvent: Sendable {
    case finishedGathering([ICloudMetadataRecord])
    case updated([ICloudMetadataRecord])
}

@MainActor
protocol ICloudMetadataQuerySource: AnyObject {
    func start(onEvent: @escaping @MainActor (ICloudMetadataQueryEvent) -> Void) -> Bool
}

@MainActor
private final class FoundationICloudMetadataQuerySource: ICloudMetadataQuerySource {
    private let query = NSMetadataQuery()
    private var observers: [NSObjectProtocol] = []
    private var didStart = false
    private var onEvent: (@MainActor (ICloudMetadataQueryEvent) -> Void)?

    func start(onEvent: @escaping @MainActor (ICloudMetadataQueryEvent) -> Void) -> Bool {
        guard !didStart else { return true }

        self.onEvent = onEvent
        query.searchScopes = [NSMetadataQueryUbiquitousDataScope]
        query.predicate = NSPredicate(value: true)

        observers.append(
            NotificationCenter.default.addObserver(
                forName: .NSMetadataQueryDidFinishGathering,
                object: query,
                queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    self.onEvent?(.finishedGathering(self.snapshot()))
                }
            }
        )
        observers.append(
            NotificationCenter.default.addObserver(
                forName: .NSMetadataQueryDidUpdate,
                object: query,
                queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    self.onEvent?(.updated(self.snapshot()))
                }
            }
        )

        guard query.start() else {
            removeObservers()
            self.onEvent = nil
            return false
        }

        didStart = true
        return true
    }

    private func snapshot() -> [ICloudMetadataRecord] {
        query.disableUpdates()
        defer { query.enableUpdates() }

        var records: [ICloudMetadataRecord] = []
        query.enumerateResults { result, _, _ in
            guard let item = result as? NSMetadataItem else { return }
            guard let name = item.value(forAttribute: NSMetadataItemFSNameKey) as? String else {
                return
            }
            guard let url = item.value(forAttribute: NSMetadataItemURLKey) as? URL else { return }

            let path =
                (item.value(forAttribute: NSMetadataItemPathKey) as? String) ?? url.path
            records.append(ICloudMetadataRecord(
                name: name,
                url: url,
                resolvedPath: Self.resolvedPath(path)
            ))
        }
        return records
    }

    private func removeObservers() {
        for observer in observers {
            NotificationCenter.default.removeObserver(observer)
        }
        observers.removeAll()
    }

    private static func resolvedPath(_ path: String) -> String {
        URL(fileURLWithPath: path).resolvingSymlinksInPath().path
    }
}

enum ICloudMetadataIndexError: Error, Equatable {
    case startFailed
    case timedOut
}

@MainActor
final class ICloudMetadataIndex {
    static let shared = ICloudMetadataIndex(source: FoundationICloudMetadataQuerySource())

    private enum Phase {
        case idle
        case gathering
        case live
    }

    private struct SnapshotWaiter {
        let continuation: CheckedContinuation<[ICloudMetadataRecord], Error>
        let timeoutTask: Task<Void, Never>
    }

    private struct ItemWaiter {
        let candidates: [ICloudMetadataCandidate]
        let continuation: CheckedContinuation<ICloudMetadataMatch, Error>
        let timeoutTask: Task<Void, Never>
    }

    private let source: ICloudMetadataQuerySource
    private var phase = Phase.idle
    private var records: [ICloudMetadataRecord] = []
    private var generation: UInt64 = 0
    private var snapshotWaiters: [UUID: SnapshotWaiter] = [:]
    private var itemWaiters: [UUID: ItemWaiter] = [:]
    private var observers: [UUID: @MainActor @Sendable () -> Void] = [:]

    init(source: ICloudMetadataQuerySource) {
        self.source = source
    }

    func currentOrInitialRecords(timeout: TimeInterval) async throws -> [ICloudMetadataRecord] {
        try Task.checkCancellation()
        try startIfNeeded()

        switch phase {
        case .live:
            return records
        case .idle, .gathering:
            return try await waitForInitialSnapshot(timeout: timeout)
        }
    }

    func settledRecords(
        timeout: TimeInterval,
        settleInterval: TimeInterval
    ) async throws -> [ICloudMetadataRecord] {
        let deadline = Date().addingTimeInterval(timeout)
        _ = try await currentOrInitialRecords(timeout: timeout)

        while true {
            try Task.checkCancellation()
            let observedGeneration = generation
            let remaining = deadline.timeIntervalSinceNow
            guard remaining > 0 else { throw ICloudMetadataIndexError.timedOut }

            try await Task.sleep(for: .seconds(min(settleInterval, remaining)))
            guard generation == observedGeneration else { continue }

            return records
        }
    }

    func itemIfPresent(
        named name: String,
        parentPath: String,
        timeout: TimeInterval
    ) async throws -> ICloudMetadataRecord? {
        try await itemsIfPresent(
            matching: [ICloudMetadataCandidate(name: name, parentPath: parentPath)],
            timeout: timeout
        ).first
    }

    func itemsIfPresent(
        matching candidates: [ICloudMetadataCandidate],
        timeout: TimeInterval
    ) async throws -> [ICloudMetadataRecord] {
        let records = try await currentOrInitialRecords(timeout: timeout)
        return Self.items(matching: candidates, in: records)
    }

    func visibleItems(matching candidates: [ICloudMetadataCandidate]) -> [ICloudMetadataRecord] {
        Self.items(matching: candidates, in: records)
    }

    func waitForItem(
        named name: String,
        parentPath: String,
        timeout: TimeInterval
    ) async throws -> ICloudMetadataRecord {
        try await waitForItems(
            matching: [ICloudMetadataCandidate(name: name, parentPath: parentPath)],
            timeout: timeout
        ).preferred
    }

    func waitForItems(
        matching candidates: [ICloudMetadataCandidate],
        timeout: TimeInterval
    ) async throws -> ICloudMetadataMatch {
        try Task.checkCancellation()
        try startIfNeeded()

        let items = Self.items(matching: candidates, in: records)
        if let match = ICloudMetadataMatch(items) {
            return match
        }

        let id = UUID()
        let match = try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                let timeoutTask = Task { @MainActor [weak self] in
                    do {
                        try await Task.sleep(for: .seconds(max(0, timeout)))
                    } catch {
                        return
                    }
                    self?.timeoutItemWaiter(id)
                }
                itemWaiters[id] = ItemWaiter(
                    candidates: candidates,
                    continuation: continuation,
                    timeoutTask: timeoutTask
                )
            }
        } onCancel: {
            Task { @MainActor [weak self] in
                self?.cancelItemWaiter(id)
            }
        }

        try Task.checkCancellation()
        return match
    }

    func addObserver(_ observer: @escaping @MainActor @Sendable () -> Void) -> UUID {
        do {
            try startIfNeeded()
        } catch {
            Log.error("metadataIndex: failed to start shared query: \(error.localizedDescription)")
        }

        let id = UUID()
        observers[id] = observer
        return id
    }

    func removeObserver(_ id: UUID) {
        observers.removeValue(forKey: id)
    }

    private func startIfNeeded() throws {
        guard case .idle = phase else { return }

        phase = .gathering
        let started = source.start { [weak self] event in
            self?.apply(event)
        }
        guard !started else { return }

        phase = .idle
        throw ICloudMetadataIndexError.startFailed
    }

    private func apply(_ event: ICloudMetadataQueryEvent) {
        switch event {
        case let .finishedGathering(records):
            self.records = records
            phase = .live
            resumeSnapshotWaiters()
        case let .updated(records):
            self.records = records
        }

        generation &+= 1
        resumeMatchingItemWaiters()

        for observer in observers.values {
            observer()
        }
    }

    private func waitForInitialSnapshot(timeout: TimeInterval) async throws -> [ICloudMetadataRecord] {
        let id = UUID()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                let timeoutTask = Task { @MainActor [weak self] in
                    do {
                        try await Task.sleep(for: .seconds(max(0, timeout)))
                    } catch {
                        return
                    }
                    self?.timeoutSnapshotWaiter(id)
                }
                snapshotWaiters[id] = SnapshotWaiter(
                    continuation: continuation,
                    timeoutTask: timeoutTask
                )
            }
        } onCancel: {
            Task { @MainActor [weak self] in
                self?.cancelSnapshotWaiter(id)
            }
        }
    }

    private func resumeSnapshotWaiters() {
        let waiters = snapshotWaiters.values
        snapshotWaiters.removeAll()
        for waiter in waiters {
            waiter.timeoutTask.cancel()
            waiter.continuation.resume(returning: records)
        }
    }

    private func resumeMatchingItemWaiters() {
        let matches = itemWaiters.compactMap { id, waiter -> (UUID, ItemWaiter, ICloudMetadataMatch)? in
            let items = Self.items(matching: waiter.candidates, in: records)
            guard let match = ICloudMetadataMatch(items) else { return nil }

            return (id, waiter, match)
        }

        for (id, waiter, match) in matches {
            itemWaiters.removeValue(forKey: id)
            waiter.timeoutTask.cancel()
            waiter.continuation.resume(returning: match)
        }
    }

    private func timeoutSnapshotWaiter(_ id: UUID) {
        guard let waiter = snapshotWaiters.removeValue(forKey: id) else { return }
        waiter.continuation.resume(throwing: ICloudMetadataIndexError.timedOut)
    }

    private func timeoutItemWaiter(_ id: UUID) {
        guard let waiter = itemWaiters.removeValue(forKey: id) else { return }
        waiter.continuation.resume(throwing: ICloudMetadataIndexError.timedOut)
    }

    private func cancelSnapshotWaiter(_ id: UUID) {
        guard let waiter = snapshotWaiters.removeValue(forKey: id) else { return }
        waiter.timeoutTask.cancel()
        waiter.continuation.resume(throwing: CancellationError())
    }

    private func cancelItemWaiter(_ id: UUID) {
        guard let waiter = itemWaiters.removeValue(forKey: id) else { return }
        waiter.timeoutTask.cancel()
        waiter.continuation.resume(throwing: CancellationError())
    }

    private static func items(
        matching candidates: [ICloudMetadataCandidate],
        in records: [ICloudMetadataRecord]
    ) -> [ICloudMetadataRecord] {
        candidates.compactMap { candidate in
            records.first {
                $0.matches(name: candidate.name, parentPath: candidate.parentPath)
            }
        }
    }
}

final class SyncHealthObserver: @unchecked Sendable {
    private let settleInterval: TimeInterval
    private let onChange: @Sendable () -> Void
    private var notifyWorkItem: DispatchWorkItem?
    private var observerID: UUID?

    init(settleInterval: TimeInterval, onChange: @escaping @Sendable () -> Void) {
        self.settleInterval = settleInterval
        self.onChange = onChange
    }

    func start() {
        Task { @MainActor [weak self] in
            guard let self else { return }
            guard observerID == nil else { return }

            observerID = ICloudMetadataIndex.shared.addObserver { [weak self] in
                self?.scheduleNotify()
            }
        }
    }

    func stop() {
        Task { @MainActor [weak self] in
            guard let self else { return }
            notifyWorkItem?.cancel()
            notifyWorkItem = nil
            if let observerID {
                ICloudMetadataIndex.shared.removeObserver(observerID)
                self.observerID = nil
            }
        }
    }

    @MainActor
    private func scheduleNotify() {
        notifyWorkItem?.cancel()
        let workItem = DispatchWorkItem { [onChange] in
            onChange()
        }
        notifyWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + settleInterval, execute: workItem)
    }
}

extension ICloudDriveHelper {
    func makeSyncHealthObserver(
        onChange: @escaping @Sendable () -> Void
    ) -> SyncHealthObserver {
        SyncHealthObserver(settleInterval: metadataSettleInterval, onChange: onChange)
    }

    func logMetadataItems(
        under parentDirectoryURL: URL,
        reason: String,
        focusName: String
    ) async {
        let resolvedParent = Self.resolvedPath(parentDirectoryURL.path)
        do {
            let records = try await metadataRecords(timeout: metadataListingTimeout)
            let matchingRecords = records.filter { $0.resolvedPath.hasPrefix(resolvedParent + "/") }
            Log.info(
                "metadataItems: reason=\(reason) focus=\(focusName) parent=\(resolvedParent) count=\(matchingRecords.count)"
            )
            for record in matchingRecords {
                Log.info(
                    "metadataItems: name=\(record.name) path=\(record.resolvedPath) url=\(record.url.path)"
                )
            }
        } catch {
            Log.info(
                "metadataItems: failed reason=\(reason) focus=\(focusName) parent=\(resolvedParent) error=\(error.localizedDescription)"
            )
        }
    }

    func waitForMetadataItem(
        named name: String,
        parentDirectoryURL: URL,
        deadline: Date
    ) async throws -> ResolvedMetadataItem {
        let resolvedParent = Self.resolvedPath(parentDirectoryURL.path)
        do {
            let record = try await metadataIndexProvider().waitForItem(
                named: name,
                parentPath: resolvedParent,
                timeout: max(0, deadline.timeIntervalSinceNow)
            )
            Log.info(
                "metadataLookup: resolved name=\(name) url=\(record.url.path) metadataPath=\(record.resolvedPath)"
            )
            return ResolvedMetadataItem(url: record.url, metadataPath: record.resolvedPath)
        } catch ICloudMetadataIndexError.startFailed {
            throw CloudStorageError.NotAvailable(
                "failed to start iCloud metadata query for \(name)"
            )
        } catch ICloudMetadataIndexError.timedOut {
            throw CloudStorageError.Offline("iCloud metadata query timed out for \(name)")
        }
    }

    func metadataItemIfPresent(
        named name: String,
        parentDirectoryURL: URL
    ) async throws -> ResolvedMetadataItem? {
        let resolvedParent = Self.resolvedPath(parentDirectoryURL.path)
        do {
            guard let record = try await metadataIndexProvider().itemIfPresent(
                named: name,
                parentPath: resolvedParent,
                timeout: metadataListingTimeout
            ) else {
                return nil
            }
            return ResolvedMetadataItem(url: record.url, metadataPath: record.resolvedPath)
        } catch ICloudMetadataIndexError.startFailed {
            throw CloudStorageError.NotAvailable(
                "failed to start iCloud metadata query for \(name)"
            )
        } catch ICloudMetadataIndexError.timedOut {
            throw CloudStorageError.Offline("iCloud metadata query timed out for \(name)")
        }
    }

    func metadataItemsIfPresent(matching urls: [URL]) async throws -> [ResolvedMetadataItem] {
        let candidates = metadataCandidates(for: urls)

        do {
            let records = try await metadataIndexProvider().itemsIfPresent(
                matching: candidates,
                timeout: metadataListingTimeout
            )
            return records.map(Self.resolvedMetadataItem)
        } catch is CancellationError {
            throw CancellationError()
        } catch ICloudMetadataIndexError.startFailed {
            throw CloudStorageError.NotAvailable("failed to start iCloud metadata query")
        } catch ICloudMetadataIndexError.timedOut {
            throw CloudStorageError.Offline("iCloud metadata query timed out")
        }
    }

    func visibleMetadataItems(matching urls: [URL]) async -> [ResolvedMetadataItem] {
        let candidates = metadataCandidates(for: urls)
        let records = await metadataIndexProvider().visibleItems(matching: candidates)
        return records.map(Self.resolvedMetadataItem)
    }

    func waitForMetadataItems(
        matching urls: [URL],
        timeout: TimeInterval,
        operation: String
    ) async throws -> ICloudMetadataMatch {
        let candidates = metadataCandidates(for: urls)

        do {
            return try await metadataIndexProvider().waitForItems(
                matching: candidates,
                timeout: timeout
            )
        } catch is CancellationError {
            throw CancellationError()
        } catch ICloudMetadataIndexError.startFailed {
            throw CloudStorageError.NotAvailable(
                "failed to start iCloud metadata query to \(operation)"
            )
        } catch ICloudMetadataIndexError.timedOut {
            throw CloudStorageError.Offline("iCloud metadata query timed out while trying to \(operation)")
        }
    }

    func metadataSubdirectoryNames(
        parentDirectoryURL: URL,
        timeout: TimeInterval
    ) async throws -> [String] {
        let records = try await metadataRecords(timeout: timeout)
        let resolvedParent = Self.resolvedPath(parentDirectoryURL.path)
        return ICloudMetadataProjection.subdirectoryNames(
            in: records,
            parentPath: resolvedParent
        )
    }

    func metadataFileNames(parentDirectoryURL: URL, prefix: String) async throws -> [String] {
        let records = try await metadataRecords(timeout: metadataListingTimeout)
        let resolvedParent = Self.resolvedPath(parentDirectoryURL.path)
        return ICloudMetadataProjection.fileNames(
            in: records,
            parentPath: resolvedParent,
            prefix: prefix
        )
    }

    private func metadataRecords(timeout: TimeInterval) async throws -> [ICloudMetadataRecord] {
        do {
            return try await metadataIndexProvider().settledRecords(
                timeout: timeout,
                settleInterval: metadataSettleInterval
            )
        } catch ICloudMetadataIndexError.startFailed {
            throw CloudStorageError.NotAvailable("failed to start iCloud metadata query")
        } catch ICloudMetadataIndexError.timedOut {
            throw CloudStorageError.Offline("iCloud metadata query timed out")
        }
    }

    private static func resolvedPath(_ path: String) -> String {
        URL(fileURLWithPath: path).resolvingSymlinksInPath().path
    }

    private func metadataCandidates(for urls: [URL]) -> [ICloudMetadataCandidate] {
        urls.map { url in
            ICloudMetadataCandidate(
                name: url.lastPathComponent,
                parentPath: Self.resolvedPath(url.deletingLastPathComponent().path)
            )
        }
    }

    private static func resolvedMetadataItem(_ record: ICloudMetadataRecord) -> ResolvedMetadataItem {
        ResolvedMetadataItem(url: record.url, metadataPath: record.resolvedPath)
    }
}
