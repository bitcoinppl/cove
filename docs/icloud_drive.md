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

This is a dev-only issue. Release/TestFlight builds with proper provisioning do not have this problem.

### Fresh or restored devices

Apple documents that metadata can arrive before file contents and that downloads may need to be triggered explicitly. In practice, that means a newly restored device may know about a file before the file data is local. Design for that case.

## FileManager

### What it can see

Once you have the ubiquity container URL, `FileManager` can enumerate what is visible locally in that container. That makes it a good fast path, but Apple still treats `NSMetadataQuery` as the guaranteed way to discover iCloud documents accurately.

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

Start with `FileManager` if you only need what is already visible in the local container and you want a fast path. Fall back to `NSMetadataQuery` when you need authoritative discovery or when the local container is still empty.

```swift
func listItems(parentPath: String) throws -> [String] {
    // fast path - use what is already visible locally
    if let names = try? listViaFileManager(parentPath), !names.isEmpty {
        return names
    }

    // authoritative fallback - ask iCloud metadata directly
    return try listViaMetadataQuery(parentPath)
}
```

### Checking whether a file exists

Use `NSMetadataQuery` with the right ubiquitous scope and a narrow predicate. That is the safer choice when local download state should not affect the answer.

### Waiting for a specific file

Use `NSMetadataQuery` with a name predicate and leave it running long enough to receive `didUpdate` events. Timeouts are app-level policy, not Apple API behavior, so pick them based on the user flow.

### Timeouts and retries in this app

These numbers are project heuristics, not Apple guidance:

- `60s` for operations that need to find one specific file, like upload or download flows
- `5s` for listing or discovery, where `FileManager` already gives us the fast path
- Onboarding cloud check uses 7 attempts with escalating delays (1, 2, 2, 3, 5, 10s) to give the iCloud daemon time to warm up on cold start
- Short timeouts with more retries work better than one long timeout because the daemon may need a few seconds after launch to initialize

## Common mistakes

1. Treating general Spotlight search-scope rules as if they are fully documented for iCloud container searches
2. Assuming `start()` is main-thread-only. The real requirement is a live run loop
3. Blocking the same run loop that should deliver query notifications
4. Calling `url(forUbiquityContainerIdentifier:)` on the main thread. It can block UI work
5. Treating placeholder filenames or file contents as stable API
6. Using a broad predicate when you already know the filename or subdirectory you want
7. Assuming metadata is available immediately at app launch. The daemon needs time to warm up, especially on first launch
8. Debugging metadata queries on a dev build with stale credentials. Bump the build number or re-sign iCloud on the device if queries return 0 results
