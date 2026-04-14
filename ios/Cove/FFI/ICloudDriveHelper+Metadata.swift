import CoveCore
import Foundation

private final class MetadataQuerySession<Value> {
    let query = NSMetadataQuery()
    let box = ICloudDriveHelper.ObserverBox()
    let semaphore = DispatchSemaphore(value: 0)
    var finalizeWorkItem: DispatchWorkItem?

    private(set) var value: Value?
    private var didSignal = false

    func finish(_ value: Value, disableUpdates: Bool = false) {
        guard !didSignal else { return }
        didSignal = true
        finalizeWorkItem?.cancel()
        if disableUpdates { query.disableUpdates() }
        self.value = value
        query.stop()
        box.removeAll()
        semaphore.signal()
    }

    func finishOnMain(_ value: Value, disableUpdates: Bool = false) {
        DispatchQueue.main.async {
            self.finish(value, disableUpdates: disableUpdates)
        }
    }

    func wait(timeout: TimeInterval) -> Value? {
        guard semaphore.wait(timeout: .now() + timeout) != .timedOut else { return nil }
        return value
    }
}

private struct OptionalMetadataQueryResult<Value> {
    let value: Value?
}

final class SyncHealthObserver {
    private let query = NSMetadataQuery()
    private let box = ICloudDriveHelper.ObserverBox()
    private let settleInterval: TimeInterval
    private let onChange: @Sendable () -> Void
    private var notifyWorkItem: DispatchWorkItem?

    init(settleInterval: TimeInterval, onChange: @escaping @Sendable () -> Void) {
        self.settleInterval = settleInterval
        self.onChange = onChange
    }

    func start() {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard !query.isStarted else { return }

            query.searchScopes = [NSMetadataQueryUbiquitousDataScope]
            query.predicate = NSPredicate(value: true)

            box.add(
                NotificationCenter.default.addObserver(
                    forName: .NSMetadataQueryDidFinishGathering,
                    object: query,
                    queue: .main
                ) { [weak self] _ in
                    self?.scheduleNotify()
                }
            )
            box.add(
                NotificationCenter.default.addObserver(
                    forName: .NSMetadataQueryDidUpdate,
                    object: query,
                    queue: .main
                ) { [weak self] _ in
                    self?.scheduleNotify()
                }
            )

            if !query.start() {
                box.removeAll()
            }
        }
    }

    func stop() {
        if Thread.isMainThread {
            stopOnMain()
            return
        }

        DispatchQueue.main.async { [weak self] in
            self?.stopOnMain()
        }
    }

    private func scheduleNotify() {
        notifyWorkItem?.cancel()
        let workItem = DispatchWorkItem { [onChange] in
            onChange()
        }
        notifyWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + settleInterval, execute: workItem)
    }

    private func stopOnMain() {
        notifyWorkItem?.cancel()
        notifyWorkItem = nil
        query.stop()
        box.removeAll()
    }
}

extension ICloudDriveHelper {
    func makeSyncHealthObserver(
        onChange: @escaping @Sendable () -> Void
    ) -> SyncHealthObserver {
        SyncHealthObserver(settleInterval: metadataSettleInterval, onChange: onChange)
    }

    // MARK: - Cloud presence via NSMetadataQuery

    private func startMetadataQuery(
        _ session: MetadataQuerySession<some Any>,
        searchScopes: [Any],
        predicate: NSPredicate,
        onStartFailure: @escaping () -> Void,
        onFinishGathering: @escaping () -> Void,
        onUpdate: @escaping () -> Void
    ) {
        DispatchQueue.main.async {
            session.query.searchScopes = searchScopes
            session.query.predicate = predicate

            session.box.add(
                NotificationCenter.default.addObserver(
                    forName: .NSMetadataQueryDidFinishGathering,
                    object: session.query,
                    queue: .main
                ) { _ in
                    onFinishGathering()
                }
            )
            session.box.add(
                NotificationCenter.default.addObserver(
                    forName: .NSMetadataQueryDidUpdate,
                    object: session.query,
                    queue: .main
                ) { _ in
                    onUpdate()
                }
            )

            if !session.query.start() {
                onStartFailure()
            }
        }
    }

    private func runMetadataResultQuery<Value, Failure: Error>(
        searchScopes: [Any],
        predicate: NSPredicate,
        timeout: TimeInterval,
        onStartFailure: @escaping (MetadataQuerySession<Result<Value, Failure>>) -> Void,
        onTimeout: @escaping (MetadataQuerySession<Result<Value, Failure>>) -> Void,
        onFinishGathering: @escaping (MetadataQuerySession<Result<Value, Failure>>) -> Void,
        onUpdate: @escaping (MetadataQuerySession<Result<Value, Failure>>) -> Void
    ) -> Result<Value, Failure>? {
        let session = MetadataQuerySession<Result<Value, Failure>>()

        startMetadataQuery(
            session,
            searchScopes: searchScopes,
            predicate: predicate,
            onStartFailure: { onStartFailure(session) },
            onFinishGathering: { onFinishGathering(session) },
            onUpdate: { onUpdate(session) }
        )

        guard let value = session.wait(timeout: timeout) else {
            onTimeout(session)
            _ = session.wait(timeout: 1)
            return nil
        }

        return value
    }

    private func runMetadataOptionalQuery<Value>(
        searchScopes: [Any],
        predicate: NSPredicate,
        timeout: TimeInterval,
        onStartFailure: @escaping (MetadataQuerySession<OptionalMetadataQueryResult<Value>>) -> Void,
        onTimeout: @escaping (MetadataQuerySession<OptionalMetadataQueryResult<Value>>) -> Void,
        onFinishGathering: @escaping (MetadataQuerySession<OptionalMetadataQueryResult<Value>>) -> Void,
        onUpdate: @escaping (MetadataQuerySession<OptionalMetadataQueryResult<Value>>) -> Void
    ) -> (didComplete: Bool, value: Value?) {
        let session = MetadataQuerySession<OptionalMetadataQueryResult<Value>>()

        startMetadataQuery(
            session,
            searchScopes: searchScopes,
            predicate: predicate,
            onStartFailure: { onStartFailure(session) },
            onFinishGathering: { onFinishGathering(session) },
            onUpdate: { onUpdate(session) }
        )

        guard let result = session.wait(timeout: timeout) else {
            onTimeout(session)
            _ = session.wait(timeout: 1)
            return (false, nil)
        }

        return (true, result.value)
    }

    /// Runs an NSMetadataQuery and returns all matching items
    ///
    /// Must NOT be called from the main thread
    func metadataQuery(
        predicate: NSPredicate,
        searchScopes: [Any] = [NSMetadataQueryUbiquitousDataScope],
        timeout: TimeInterval? = nil
    ) throws -> [NSMetadataItem] {
        let effectiveTimeout = timeout ?? defaultTimeout
        let finishQuery = {
            (session: MetadataQuerySession<Result<[NSMetadataItem], CloudStorageError>>, reason: String)
            in
            let results = (0 ..< session.query.resultCount).compactMap {
                session.query.result(at: $0) as? NSMetadataItem
            }
            Log.info(
                "metadataQuery: finalized reason=\(reason) count=\(results.count) predicate=\(predicate.predicateFormat)"
            )
            session.finish(.success(results), disableUpdates: true)
        }

        Log.info("metadataQuery: starting predicate=\(predicate.predicateFormat)")
        let result = runMetadataResultQuery(
            searchScopes: searchScopes,
            predicate: predicate,
            timeout: effectiveTimeout,
            onStartFailure: { session in
                session.box.removeAll()
                session.finish(.failure(.NotAvailable("failed to start iCloud metadata query")))
            },
            onTimeout: { session in
                session.finishOnMain(.failure(.Offline("iCloud metadata query timed out")))
            },
            onFinishGathering: {
                (session: MetadataQuerySession<Result<[NSMetadataItem], CloudStorageError>>) in
                Log.info(
                    "metadataQuery: finish gathering count=\(session.query.resultCount) predicate=\(predicate.predicateFormat)"
                )
                self.scheduleFinalize(session, finishQuery: finishQuery, reason: "finish")
            },
            onUpdate: {
                (session: MetadataQuerySession<Result<[NSMetadataItem], CloudStorageError>>) in
                Log.info(
                    "metadataQuery: update count=\(session.query.resultCount) predicate=\(predicate.predicateFormat)"
                )
                self.scheduleFinalize(session, finishQuery: finishQuery, reason: "update")
            }
        )

        guard let result else { throw CloudStorageError.Offline("iCloud metadata query timed out") }

        switch result {
        case let .success(results):
            return results
        case let .failure(error):
            throw error
        }
    }

    private func scheduleFinalize<Value>(
        _ session: MetadataQuerySession<Value>,
        finishQuery: @escaping (MetadataQuerySession<Value>, String) -> Void,
        reason: String
    ) {
        DispatchQueue.main.async {
            let scheduleFinalize = { (reason: String) in
                session.finalizeWorkItem?.cancel()
                let workItem = DispatchWorkItem {
                    finishQuery(session, reason)
                }
                session.finalizeWorkItem = workItem
                DispatchQueue.main.asyncAfter(
                    deadline: .now() + self.metadataSettleInterval,
                    execute: workItem
                )
            }
            scheduleFinalize(reason)
        }
    }

    /// Authoritatively checks whether a file exists in iCloud (finds evicted files too)
    ///
    /// Must NOT be called from the main thread
    func fileExistsInCloud(name: String) throws -> Bool {
        let predicate = NSPredicate(format: "%K == %@", NSMetadataItemFSNameKey, name)
        let results = try metadataQuery(predicate: predicate)
        return !results.isEmpty
    }

    /// Resolve symlinks so /var and /private/var compare correctly
    private static func resolvedPath(_ path: String) -> String {
        URL(fileURLWithPath: path).resolvingSymlinksInPath().path
    }

    private static func metadataPath(for item: NSMetadataItem) -> String? {
        if let path = item.value(forAttribute: NSMetadataItemPathKey) as? String { return resolvedPath(path) }
        if let url = item.value(forAttribute: NSMetadataItemURLKey) as? URL { return resolvedPath(url.path) }
        return nil
    }

    private static func resolvedItem(
        named name: String,
        under resolvedParent: String,
        in query: NSMetadataQuery
    ) -> ResolvedMetadataItem? {
        let prefix = resolvedParent + "/"

        for index in 0 ..< query.resultCount {
            guard let item = query.result(at: index) as? NSMetadataItem else { continue }
            guard let itemName = item.value(forAttribute: NSMetadataItemFSNameKey) as? String else { continue }
            guard itemName == name else { continue }
            guard let metadataURL = item.value(forAttribute: NSMetadataItemURLKey) as? URL else { continue }
            let metadataPath = Self.metadataPath(for: item)
            if let metadataPath, metadataPath.hasPrefix(prefix) {
                return ResolvedMetadataItem(url: metadataURL, metadataPath: metadataPath)
            }
        }

        return nil
    }

    private static func metadataItemSummary(_ item: NSMetadataItem) -> String {
        let name = (item.value(forAttribute: NSMetadataItemFSNameKey) as? String) ?? "<unknown>"
        let path = metadataPath(for: item) ?? "<no-path>"
        let url =
            ((item.value(forAttribute: NSMetadataItemURLKey) as? URL)?.path) ?? "<no-url>"
        return "name=\(name) path=\(path) url=\(url)"
    }

    private static func metadataItemSummaries(in query: NSMetadataQuery) -> [String] {
        (0 ..< query.resultCount).compactMap { index in
            guard let item = query.result(at: index) as? NSMetadataItem else {
                return nil
            }
            return metadataItemSummary(item)
        }
    }

    func logMetadataItems(
        under parentDirectoryURL: URL,
        reason: String,
        focusName: String
    ) {
        let resolvedParent = Self.resolvedPath(parentDirectoryURL.path)
        let finish = { (session: MetadataQuerySession<OptionalMetadataQueryResult<Void>>) in
            let summaries = Self.metadataItemSummaries(in: session.query)
            Log.info(
                "metadataItems: reason=\(reason) focus=\(focusName) parent=\(resolvedParent) count=\(summaries.count)"
            )
            for summary in summaries {
                Log.info("metadataItems: \(summary)")
            }
            session.finish(OptionalMetadataQueryResult(value: ()))
        }

        let result = runMetadataOptionalQuery(
            searchScopes: [NSMetadataQueryUbiquitousDataScope],
            predicate: NSPredicate(value: true),
            timeout: 5,
            onStartFailure: { session in
                Log.info(
                    "metadataItems: failed to start reason=\(reason) focus=\(focusName) parent=\(resolvedParent)"
                )
                session.box.removeAll()
                session.finish(OptionalMetadataQueryResult(value: ()))
            },
            onTimeout: { session in
                session.finishOnMain(OptionalMetadataQueryResult(value: ()))
            },
            onFinishGathering: finish,
            onUpdate: finish
        )

        guard result.didComplete else {
            Log.info(
                "metadataItems: timed out reason=\(reason) focus=\(focusName) parent=\(resolvedParent)"
            )
            return
        }
    }

    func waitForMetadataItem(
        named name: String,
        parentDirectoryURL: URL,
        deadline: Date
    ) throws -> ResolvedMetadataItem {
        let resolvedParent = Self.resolvedPath(parentDirectoryURL.path)
        let predicate = NSPredicate(format: "%K == %@", NSMetadataItemFSNameKey, name)
        let finish = {
            (
                session: MetadataQuerySession<Result<ResolvedMetadataItem, MetadataLookupError>>,
                item: ResolvedMetadataItem?,
                error: MetadataLookupError?
            ) in
            if let error {
                session.finish(.failure(error))
                return
            }

            if let item {
                session.finish(.success(item))
                return
            }

            session.finish(
                .failure(.missingURL("iCloud metadata query finished without a URL for \(name)"))
            )
        }

        let evaluate = {
            (
                session: MetadataQuerySession<Result<ResolvedMetadataItem, MetadataLookupError>>,
                reason: String
            ) in
            if let item = Self.resolvedItem(
                named: name,
                under: resolvedParent,
                in: session.query
            ) {
                Log.info(
                    "metadataLookup: resolved name=\(name) reason=\(reason) url=\(item.url.path) metadataPath=\(item.metadataPath ?? "<unknown>")"
                )
                finish(session, item, nil)
                return
            }

            Log.info(
                "metadataLookup: no match yet name=\(name) reason=\(reason) count=\(session.query.resultCount) parent=\(resolvedParent)"
            )
            for summary in Self.metadataItemSummaries(in: session.query) {
                Log.info("metadataLookup: item \(summary)")
            }
        }

        Log.info(
            "metadataLookup: starting name=\(name) parent=\(resolvedParent) predicate=\(predicate.predicateFormat)"
        )
        let result = runMetadataResultQuery(
            searchScopes: [NSMetadataQueryUbiquitousDataScope],
            predicate: predicate,
            timeout: deadline.timeIntervalSinceNow,
            onStartFailure: { session in
                finish(
                    session,
                    nil,
                    .startFailed("failed to start iCloud metadata query for \(name)")
                )
            },
            onTimeout: { session in
                session.finishOnMain(
                    .failure(.timedOut("iCloud metadata query timed out for \(name)"))
                )
            },
            onFinishGathering: { session in
                evaluate(session, "finish")
            },
            onUpdate: { session in
                evaluate(session, "update")
            }
        )

        guard let result else { throw MetadataLookupError.timedOut("iCloud metadata query timed out for \(name)") }

        switch result {
        case let .failure(failure):
            throw failure
        case let .success(resolvedItem):
            return resolvedItem
        }
    }

    func resolvedMetadataItemIfPresent(
        named name: String,
        parentDirectoryURL: URL
    ) -> ResolvedMetadataItem? {
        let resolvedParent = Self.resolvedPath(parentDirectoryURL.path)
        let predicate = NSPredicate(format: "%K == %@", NSMetadataItemFSNameKey, name)

        let result: (didComplete: Bool, value: ResolvedMetadataItem?) = runMetadataOptionalQuery(
            searchScopes: [NSMetadataQueryUbiquitousDataScope],
            predicate: predicate,
            timeout: 5,
            onStartFailure: { session in
                session.finish(OptionalMetadataQueryResult(value: nil))
            },
            onTimeout: { session in
                session.finishOnMain(OptionalMetadataQueryResult(value: nil))
            },
            onFinishGathering: { session in
                let match = Self.resolvedItem(
                    named: name,
                    under: resolvedParent,
                    in: session.query
                )
                session.finish(OptionalMetadataQueryResult(value: match))
            },
            onUpdate: { session in
                let match = Self.resolvedItem(
                    named: name,
                    under: resolvedParent,
                    in: session.query
                )
                session.finish(OptionalMetadataQueryResult(value: match))
            }
        )

        guard result.didComplete else { return nil }
        return result.value
    }

    func metadataItemIfPresent(
        named name: String,
        parentDirectoryURL: URL
    ) throws -> ResolvedMetadataItem? {
        let resolvedParent = Self.resolvedPath(parentDirectoryURL.path)
        let predicate = NSPredicate(format: "%K == %@", NSMetadataItemFSNameKey, name)
        let items = try metadataQuery(
            predicate: predicate,
            searchScopes: [NSMetadataQueryUbiquitousDataScope],
            timeout: 5
        )

        for item in items {
            guard
                let match = Self.resolvedMetadataItem(
                    from: item,
                    named: name,
                    under: resolvedParent
                )
            else {
                continue
            }
            return match
        }

        return nil
    }

    private func metadataNames(
        parentDirectoryURL: URL,
        transform: (String) -> String?
    ) throws -> [String] {
        let resolvedParent = Self.resolvedPath(parentDirectoryURL.path)
        let pathPrefix = resolvedParent + "/"
        let items = try metadataQuery(
            predicate: NSPredicate(value: true),
            timeout: 5
        )
        var names = Set<String>()

        for item in items {
            guard let metadataPath = Self.metadataPath(for: item) else { continue }
            guard metadataPath.hasPrefix(pathPrefix) else { continue }

            let relativePath = String(metadataPath.dropFirst(pathPrefix.count))
            guard let name = transform(relativePath) else { continue }
            names.insert(name)
        }

        return names.sorted()
    }

    func metadataSubdirectoryNames(parentDirectoryURL: URL) throws -> [String] {
        try metadataNames(parentDirectoryURL: parentDirectoryURL) { relativePath in
            guard let firstComponent = relativePath.split(separator: "/").first else { return nil }
            return String(firstComponent)
        }
    }

    func metadataFileNames(parentDirectoryURL: URL, prefix: String) throws -> [String] {
        try metadataNames(parentDirectoryURL: parentDirectoryURL) { relativePath in
            guard !relativePath.contains("/") else { return nil }
            let name = URL(fileURLWithPath: relativePath).lastPathComponent
            guard name.hasPrefix(prefix) else { return nil }
            return name
        }
    }

    private static func resolvedMetadataItem(
        from item: NSMetadataItem,
        named name: String,
        under resolvedParent: String
    ) -> ResolvedMetadataItem? {
        let prefix = resolvedParent + "/"
        guard let itemName = item.value(forAttribute: NSMetadataItemFSNameKey) as? String else { return nil }
        guard itemName == name else { return nil }
        guard let metadataURL = item.value(forAttribute: NSMetadataItemURLKey) as? URL else { return nil }
        let metadataPath = Self.metadataPath(for: item)
        guard let metadataPath, metadataPath.hasPrefix(prefix) else { return nil }

        return ResolvedMetadataItem(url: metadataURL, metadataPath: metadataPath)
    }
}
