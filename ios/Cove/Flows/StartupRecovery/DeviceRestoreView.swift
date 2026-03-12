import SwiftUI

@_exported import CoveCore

struct DeviceRestoreView: View {
    let onResolve: () -> Void

    @State private var cloudBackupState: CloudBackupCheckState = .checking
    @State private var restoreInProgress = false
    @State private var restoreError: String?
    @State private var showStartFreshConfirmation = false
    @State private var partialRestoreReport: CloudBackupRestoreReport?
    @State private var manager = RustCloudBackupManager()

    enum CloudBackupCheckState {
        case checking
        case found
        case absent
        case unavailable(String)
    }

    var body: some View {
        VStack(spacing: 24) {
            Spacer()

            Image(systemName: "icloud.and.arrow.down")
                .font(.system(size: 64))
                .foregroundStyle(.blue)

            Text("Device Restored")
                .font(.title)
                .fontWeight(.bold)

            Text(
                "Your device was restored from a backup, but the encryption keys for your wallet data weren't included."
            )
            .multilineTextAlignment(.center)
            .foregroundStyle(.secondary)
            .padding(.horizontal, 32)

            Spacer()

            switch cloudBackupState {
            case .checking:
                ProgressView("Checking for cloud backup...")

            case .found:
                VStack(spacing: 16) {
                    Button {
                        restoreFromCloud()
                    } label: {
                        HStack {
                            if restoreInProgress {
                                ProgressView()
                                    .tint(.white)
                            }
                            Text("Restore from Cloud Backup")
                        }
                        .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(restoreInProgress)

                    Button("Start Fresh") {
                        showStartFreshConfirmation = true
                    }
                    .foregroundStyle(.secondary)
                }

            case .absent:
                VStack(spacing: 16) {
                    Text("No cloud backup found.")
                        .foregroundStyle(.secondary)

                    Button {
                        showStartFreshConfirmation = true
                    } label: {
                        Text("Start Fresh")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                }

            case let .unavailable(message):
                VStack(spacing: 16) {
                    Text("Unable to check for cloud backup")
                        .font(.headline)

                    Text(message)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)

                    Button("Retry") {
                        checkForCloudBackup()
                    }
                    .buttonStyle(.borderedProminent)

                    Button("Start Fresh") {
                        showStartFreshConfirmation = true
                    }
                    .foregroundStyle(.secondary)
                }
            }

            if let restoreError {
                Text(restoreError)
                    .foregroundStyle(.red)
                    .font(.caption)
                    .padding(.horizontal)
            }

            Spacer()
        }
        .padding()
        .onAppear {
            checkForCloudBackup()
        }
        .alert("Start Fresh?", isPresented: $showStartFreshConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Start Fresh", role: .destructive) {
                startFresh()
            }
        } message: {
            Text(
                "This will remove all existing wallet data and start with a clean installation. This cannot be undone."
            )
        }
        .alert(
            "Partial Restore",
            isPresented: Binding(
                get: { partialRestoreReport != nil },
                set: { if !$0 { partialRestoreReport = nil } }
            )
        ) {
            Button("Continue") {
                createSentinelIfNeeded()
                onResolve()
            }
        } message: {
            if let report = partialRestoreReport {
                Text(
                    "\(report.walletsRestored) of \(report.walletsRestored + report.walletsFailed) wallets restored. \(report.walletsFailed) failed."
                )
            }
        }
    }

    private func checkForCloudBackup() {
        cloudBackupState = .checking

        Task.detached {
            do {
                let cloudStorage = CloudStorageAccessImpl()
                let hasBackup = try cloudStorage.hasCloudBackup()

                await MainActor.run {
                    cloudBackupState = hasBackup ? .found : .absent
                }
            } catch {
                await MainActor.run {
                    cloudBackupState = .unavailable(error.localizedDescription)
                }
            }
        }
    }

    private func restoreFromCloud() {
        restoreInProgress = true
        restoreError = nil

        manager.listenForUpdates(reconciler: RestoreReconciler { message in
            Task { @MainActor in
                handleRestoreMessage(message)
            }
        })

        manager.restoreFromCloudBackup()
    }

    private func handleRestoreMessage(_ message: CloudBackupReconcileMessage) {
        switch message {
        case let .restoreComplete(report):
            restoreInProgress = false
            if report.walletsFailed > 0 {
                partialRestoreReport = report
            } else {
                createSentinelIfNeeded()
                onResolve()
            }
        case let .stateChanged(state):
            if case let .error(msg) = state {
                restoreError = msg
                restoreInProgress = false
            }
        case .progressUpdated, .enableComplete:
            break
        }
    }

    private func startFresh() {
        wipeLocalData()
        createSentinelIfNeeded()
        onResolve()
    }
}

private class RestoreReconciler: CloudBackupManagerReconciler {
    let handler: (CloudBackupReconcileMessage) -> Void

    init(handler: @escaping (CloudBackupReconcileMessage) -> Void) {
        self.handler = handler
    }

    func reconcile(message: CloudBackupReconcileMessage) {
        handler(message)
    }
}

func createSentinelIfNeeded() {
    let path = sentinelPath()
    let fileURL = URL(fileURLWithPath: path)

    guard !FileManager.default.fileExists(atPath: path) else { return }

    // create the file
    guard FileManager.default.createFile(atPath: path, contents: Data()) else {
        Log.error("Failed to create sentinel file")
        return
    }

    // set isExcludedFromBackup atomically
    do {
        var url = fileURL
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try url.setResourceValues(values)
    } catch {
        Log.error("Failed to set isExcludedFromBackup on sentinel: \(error)")
        // remove the file to avoid a partial state
        try? FileManager.default.removeItem(at: fileURL)
    }
}
