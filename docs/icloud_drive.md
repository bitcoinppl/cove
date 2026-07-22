# Notes on iCloud Drive

Things we learned the hard way about iCloud Drive and ubiquity containers on iOS. If you're changing file discovery or metadata query code, read this first.

This note keeps Apple-documented behavior separate from project heuristics where it matters.

## NSMetadataQuery

### Search scopes

Apple documents two search scopes for iCloud document queries in an app container:

- `NSMetadataQueryUbiquitousDataScope` searches files outside `Documents/`
- `NSMetadataQueryUbiquitousDocumentsScope` searches files inside `Documents/`

For this codebase, prefer those scopes for iCloud lookups. Apple documents directory and URL scopes for general Spotlight queries, but does not document directory URL scopes as the way to search an iCloud ubiquity container.

```swift
// avoid for iCloud lookups in this project
query.searchScopes = [someDirectoryURL]

// preferred
query.searchScopes = [NSMetadataQueryUbiquitousDataScope]
```

If you need one subdirectory, start with the appropriate iCloud scope and narrow the results with a predicate or path filtering in code.

### Threading and lifecycle

- `NSMetadataQuery` is asynchronous and needs a live run loop to produce results
- `NSMetadataQueryDidFinishGathering` gives you the initial snapshot
- `NSMetadataQueryDidUpdate` reports later changes
- Starting on the main thread is the simplest option because the main run loop is already running
- A background-thread query is still valid, but you have to run that thread's run loop yourself
- Do not block the same run loop you expect to deliver query notifications
- Keep `url(forUbiquityContainerIdentifier:)` off the main thread; Apple says it may take a nontrivial amount of time

Cove owns one main-actor metadata query for the lifetime of the process. The query searches the ubiquitous data scope, publishes value snapshots to async consumers, and remains running so later `didUpdate` events can satisfy upload-confirmation checks, downloads, listings, and sync-health checks. Do not create short-lived queries for individual operations or call `stop()` during normal app operation. Tearing down a query while CloudDocs is delivering progress notifications can race inside `NSMetadataQuery` cleanup.

The initial result is authoritative only after `NSMetadataQueryDidFinishGathering`. Updates received while gathering may reveal an item early, but an empty partial snapshot must not be interpreted as proof that an item is absent. A transient `start()` failure is not process-fatal: the shared index surfaces it to the operation that attempted startup and permits a later consumer to retry.

### Cold start timing

The iCloud daemon needs time after app launch to initialize its metadata index. On a fresh launch, `NSMetadataQuery` may return 0 results even though `FileManager` can see files in the ubiquity container. On subsequent launches the daemon is already warm and metadata queries work immediately.

We confirmed this by running metadata queries in the background after FileManager returned results: attempt 1 on fresh launch found nothing, but on second app launch it worked instantly.

### Development builds

In dev builds, the iCloud daemon can fail to authenticate, producing these errors:

```text
[ERROR] couldn't fetch remote operation IDs: NSError: Cocoa 257
"Error returned from daemon: Error Domain=com.apple.accounts Code=7 "(null)""
```

When this happens, `NSMetadataQuery` returns 0 results but `FileManager` still works (it reads the local filesystem directly). Bumping the build number in Xcode can fix this by regenerating the provisioning profile and clearing stale credentials. Signing out of iCloud and back in on the device also works.

Treat this as a signing, entitlement, or account-state diagnostic rather than as
proof that the backup is empty. Release and TestFlight builds should still be
verified with their actual entitlements and iCloud account state.

### Fresh or restored devices

Apple documents that metadata can arrive before file contents and that downloads may need to be triggered explicitly. In practice, that means a newly restored device may know about a file before the file data is local. Design for that case.

## FileManager

### What it can see

Once you have the ubiquity container URL, `FileManager` can enumerate what is visible locally in that container. That makes it a good fast path, but Apple still treats `NSMetadataQuery` as the guaranteed way to discover iCloud documents accurately.

For user-facing backup inventory, a nonempty FileManager result is still only a
local snapshot. It must not short-circuit the metadata query because evicted or
late-visible items may be missing locally.

## NSFileCoordinator

### Why this app uses it

For iCloud-backed files, coordination is the safe path for actual reads, writes, deletes, and directory creation. The system may need to serialize access with another process, hand back a different concrete URL than the one you started with, or materialize file contents that are still represented by a placeholder.

In this project, the coordinated helpers in `ICloudDriveHelper` are the default for touching file contents in the ubiquity container. Direct `Data(contentsOf:)`, `data.write(to:)`, or `FileManager.removeItem` calls are fine for ordinary local files, but they are less reliable for ubiquitous items.

### How to use it correctly

- use the URL passed into the coordination closure, not the original URL
- treat the outer coordinator error separately from the inner read or write error
- keep metadata discovery and file coordination as separate concerns: `NSMetadataQuery` finds the item, `NSFileCoordinator` safely touches it
- a coordinated read can also trigger download or materialization of an evicted iCloud file

### Placeholders and download state

A metadata query can return placeholder items before the file contents are local. The actual data is downloaded when one of these happens:

- your app opens or otherwise accesses the file
- your app calls `startDownloadingUbiquitousItem`

Use URL resource values to check download state:

- `NSURLUbiquitousItemIsDownloadedKey` tells you whether the item is local
- `NSURLUbiquitousItemDownloadingStatusKey` gives you more detail about download state

### Placeholder formats

Apple documents placeholder behavior and download-status keys, but not the exact on-disk representation. Do not build long-term logic around placeholder filenames or private file formats.

## Recommended patterns

### Listing files or directories

For user-facing inventory, take a fast FileManager snapshot and always run the
normalized metadata query before treating an inventory as complete. The complete
result is the sorted union. If the metadata query fails or times out, the complete
listing throws instead of returning its local rows or reporting confirmed zero.
A caller may continue showing a separately retained local snapshot as provisional
data, but that snapshot remains explicitly incomplete.

```swift
func listItems(parentPath: String) throws -> [String] {
    let local = (try? listViaFileManager(parentPath)) ?? []
    let metadata = try listViaMetadataQuery(parentPath)
    let normalized = (local + metadata).map { name in
        guard name.hasPrefix("."), name.hasSuffix(".icloud") else { return name }
        return String(name.dropFirst().dropLast(".icloud".count))
    }
    return Array(Set(normalized)).sorted()
}
```

The private `CloudStorageAccessImpl.listSubdirectories` and `listFiles` methods
implement this union for complete namespace and wallet-file listings. A metadata
failure makes those operations throw, so they do not return a partial union.
`CloudStorageAccessImpl.listWalletFilesSnapshot` deliberately returns only the
best-effort local result with `isComplete = false`; Rust may publish or retain it
while the mandatory complete listing continues.

### Checking whether a file exists

Read the shared metadata index and filter its snapshot by resolved parent path and filename. This is safer than checking only the local filesystem when download state should not affect the answer.

### Waiting for a specific file

Wait on the shared metadata index for a matching `didUpdate` snapshot. Timeouts are app-level policy, not Apple API behavior, so pick them based on the user flow.

### Uploading files

`CloudStorageAccessImpl.uploadMasterKeyBackup` and `uploadWalletBackup` return
after `ICloudDriveHelper.writeForUpload` validates the bytes staged in the local
iCloud container. That handoff is intentionally asynchronous: it does not prove
that the provider can see the file or that the remote revision is current.

Rust owns upload confirmation. Its pending-upload worker later calls
`isBackupUploaded`, then downloads the backup and verifies its revision before
marking the blob confirmed. Do not make the upload methods wait for metadata
visibility; provider confirmation belongs to that retryable background flow.

### Timeouts and retries in this app

These numbers are project heuristics, not Apple guidance:

- `60s` for operations that need to find one specific file, such as downloads
- `5s` for a normal metadata listing while FileManager supplies the fast local snapshot
- silent onboarding namespace discovery has one outer `15s` deadline, performs at most four inspections, and uses retry delays of `1s`, `2s`, and `4s`
- each silent inspection caps its metadata query at `5s` and leaves cleanup time inside the outer deadline
- cancellation stops queued silent work; an expired deadline is unavailable or incomplete, never confirmed empty

The outer deadline applies to silent discovery, not to interactive passkey or
provider sheets. Rust does not restart silent discovery after that deadline or
another storage failure; its bounded retries apply only after namespace
metadata is visible while recovery files are still pending. Interactive passkey
restore separately re-polls namespace discovery while user-authorized work
remains active.

## Common mistakes

1. Treating general Spotlight search-scope rules as if they are fully documented for iCloud container searches
2. Assuming `start()` is main-thread-only. The real requirement is a live run loop
3. Blocking the same run loop that should deliver query notifications
4. Calling `url(forUbiquityContainerIdentifier:)` on the main thread. It can block UI work
5. Treating placeholder filenames or file contents as stable API
6. Creating and tearing down a metadata query for every filename or subdirectory lookup
7. Assuming metadata is available immediately at app launch. The daemon needs time to warm up, especially on first launch
8. Debugging metadata queries on a dev build with stale credentials. Bump the build number or re-sign iCloud on the device if queries return 0 results
9. Returning a nonempty FileManager snapshot without running the mandatory metadata union
10. Converting metadata timeout or failure into a confirmed empty inventory
