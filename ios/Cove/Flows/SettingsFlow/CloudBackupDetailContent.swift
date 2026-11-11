import SwiftUI

/// The detail screen's body, one branch for each body Rust projects
struct CloudBackupDetailBody: View {
    let screen: CloudBackupScreen
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    var body: some View {
        CloudBackupDetailNotices(screen: screen, manager: manager)

        switch screen.body {
        case let .configured(configured):
            CloudBackupConfiguredSections(
                screen: screen,
                configured: configured,
                manager: manager,
                presentationCoordinator: presentationCoordinator
            )

        case let .finishingDisable(finishing):
            CloudBackupFinishingDisableSection(
                stage: finishing.stage,
                actions: screen.actions,
                manager: manager
            )

        case .loadFailed:
            CloudBackupLoadFailedSection(actions: screen.actions, manager: manager)

        case .corrupted:
            CloudBackupCorruptedSection(actions: screen.actions, manager: manager)

        case .enabling, .restoring:
            CloudBackupLoadingSection()

        case .pendingEnableRecovery:
            CloudBackupInterruptedSetupSection()

        // the screen dismisses itself when Cloud Backup is off, and drive account switching is
        // Android-only
        case .disabled, .driveAccountSwitch:
            EmptyView()
        }
    }
}

/// The heal notice, and a last outcome whose control isn't on screen
private struct CloudBackupDetailNotices: View {
    let screen: CloudBackupScreen
    let manager: CloudBackupManager

    var body: some View {
        if let notice = screen.healNotice {
            CloudBackupHealNoticeSection(notice: notice, actions: screen.actions, manager: manager)
        }

        if let outcome = screen.unplacedOutcome {
            Section {
                CloudBackupOutcomeNotice(outcome: outcome) {
                    manager.dispatch(action: .dismissNotice(.lastOutcome))
                }
            }
        }
    }
}

/// The configured screen's sections, in the order Rust lists them
private struct CloudBackupConfiguredSections: View {
    let screen: CloudBackupScreen
    let configured: CloudBackupConfiguredScreen
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    var body: some View {
        ForEach(configured.sections, id: \.kind) { section in
            CloudBackupSectionContent(
                section: section,
                screen: screen,
                configured: configured,
                manager: manager,
                presentationCoordinator: presentationCoordinator
            )
        }
    }
}

private struct CloudBackupSectionContent: View {
    let section: CloudBackupSection
    let screen: CloudBackupScreen
    let configured: CloudBackupConfiguredScreen
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    var body: some View {
        switch section {
        case let .passkeyCard(state):
            CloudBackupPasskeyCard(state: state, actions: screen.actions, manager: manager)

        case let .progress(progress):
            CloudBackupProgressContent(progress: progress, actions: screen.actions, manager: manager)

        case let .verification(verification):
            VerificationSection(
                section: verification,
                actions: screen.actions,
                passkeyRepair: configured.passkeyCard?.repairState,
                wallets: configured.wallets,
                manager: manager,
                presentationCoordinator: presentationCoordinator
            )

        case let .wallets(wallets):
            CloudBackupWalletsContent(
                wallets: wallets,
                configured: configured,
                actions: screen.actions,
                manager: manager
            )

        case let .notOnDevice(notOnDevice):
            CloudBackupNotOnDeviceSectionView(
                section: notOnDevice,
                actions: screen.actions,
                manager: manager,
                presentationCoordinator: presentationCoordinator
            )

        case let .extraCopies(extraCopies):
            CloudBackupExtraCopiesSectionView(
                section: extraCopies,
                manager: manager,
                presentationCoordinator: presentationCoordinator
            )

        case let .otherBackups(otherBackups):
            OtherBackupsSection(
                section: otherBackups,
                actions: screen.actions,
                manager: manager,
                presentationCoordinator: presentationCoordinator
            )

        case .turnOff:
            CloudBackupTurnOffSection(
                disable: configured.disable,
                actions: screen.actions,
                manager: manager
            )
        }
    }
}

/// This device's wallets, with the Sync Now Rust names when some are waiting to upload
private struct CloudBackupWalletsContent: View {
    let wallets: CloudBackupWalletsSection
    let configured: CloudBackupConfiguredScreen
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    var body: some View {
        HeaderSection(lastSync: configured.lastSync, syncHealth: configured.syncHealth)

        let items = (wallets.upToDate + wallets.needsSync).map(\.wallet)
        if !items.isEmpty {
            WalletSections(wallets: items)
        }

        ForEach(wallets.controls, id: \.self) { control in
            switch control {
            case .syncNow:
                Section {
                    CloudBackupSyncNowControl(
                        syncState: wallets.sync,
                        actions: actions,
                        onSync: { manager.dispatch(action: .syncUnsynced) }
                    )
                }
            }
        }
    }
}

private struct CloudBackupProgressContent: View {
    let progress: CloudBackupProgressSection
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    var body: some View {
        switch progress {
        case .inventoryChecking:
            CloudBackupInventoryCheckingSection()
        case let .inventoryIncomplete(reason, controls):
            CloudBackupInventoryIncompleteSection(
                reason: reason,
                controls: controls,
                actions: actions,
                manager: manager
            )
        case .verificationCard:
            CloudBackupVerificationCard()
        case .verificationInline:
            CloudBackupVerificationInlineSection()
        }
    }
}

private struct CloudBackupInventoryCheckingSection: View {
    var body: some View {
        Section {
            HStack {
                ProgressView()
                    .padding(.trailing, 8)

                Text("Refreshing the wallet backup list...")
            }
            .foregroundStyle(.secondary)
            .accessibilityIdentifier("cloudBackup.inventory.checking")
        }
    }
}

private struct CloudBackupInventoryIncompleteSection: View {
    let reason: CloudBackupInventoryIncompleteReason
    let controls: [CloudBackupInventoryControl]
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    @AccessibilityFocusState private var isFocused: Bool

    var body: some View {
        Section {
            Label("Cloud backup inventory is incomplete", systemImage: "exclamationmark.icloud")
                .foregroundStyle(Color.statusError)
                .accessibilityFocused($isFocused)
                .accessibilityIdentifier("cloudBackup.inventory.incomplete")

            Text(reason.inventoryMessage)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            ForEach(controls, id: \.self) { control in
                switch control {
                case .checkAgain:
                    CloudBackupActionControl(
                        action: .refreshDetail,
                        actions: actions,
                        perform: { manager.dispatch(action: .refreshDetail) }
                    ) {
                        Label("Check Again", systemImage: "arrow.clockwise")
                    }

                // only Google Drive asks to be authorized again; iCloud never does
                case .reconnectDrive:
                    EmptyView()
                }
            }
        }
        .onAppear { isFocused = true }
    }
}

private struct CloudBackupVerificationCard: View {
    var body: some View {
        Section {
            VStack {
                ProgressView("Verifying cloud backup...")
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
        }
    }
}

private struct CloudBackupVerificationInlineSection: View {
    var body: some View {
        Section {
            HStack {
                ProgressView()
                    .padding(.trailing, 8)

                Text("Verifying backup integrity...")
            }
        }
    }
}

private struct CloudBackupLoadingSection: View {
    var body: some View {
        Section {
            VStack(spacing: 12) {
                ProgressView("Loading cloud backup...")

                Text("Finishing setup and fetching backup details")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
        }
    }
}

private struct CloudBackupInterruptedSetupSection: View {
    var body: some View {
        Section {
            ProbedLabel("Cloud Backup setup was interrupted", systemImage: "exclamationmark.icloud")
                .foregroundStyle(Color.statusWarning)

            Text("Go back to Settings to review the interrupted setup.")
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(.secondary)
        }
    }
}

/// Shown after Cove found a Turn Off that never finished and turned backup back on
private struct CloudBackupHealNoticeSection: View {
    let notice: CloudBackupHealNotice
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    var body: some View {
        Section {
            ProbedLabel("Cloud Backup is on again", systemImage: "checkmark.icloud")
                .foregroundStyle(Color.statusInfo)
                .accessibilityIdentifier("cloudBackup.healNotice")

            Text(
                "An earlier Turn Off didn't finish, so Cove turned Cloud Backup back on. Cove is backing up your wallets again."
            )
            .font(.caption)
            .uiTestTruncationProbe()
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            CloudBackupActionControl(
                action: .turnOff,
                actions: actions,
                role: .destructive,
                identifier: "cloudBackup.healNotice.turnOff",
                perform: { manager.dispatch(action: .disableCloudBackup) }
            ) {
                ProbedLabel("Turn Off Cloud Backup", systemImage: "icloud.slash")
            }

            CloudBackupActionControl(
                action: .dismissHealNotice,
                actions: actions,
                perform: { manager.dispatch(action: .dismissNotice(.heal)) }
            ) {
                Text("Dismiss").uiTestTruncationProbe()
            }
        }
    }
}

/// A committed Turn Off that couldn't finish deleting the backups
private struct CloudBackupFinishingDisableSection: View {
    let stage: CloudBackupFinishingDisableStage
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    var body: some View {
        Section {
            ProbedLabel("Cloud Backup is being turned off", systemImage: "icloud.slash")
                .foregroundStyle(Color.statusWarning)
                .accessibilityIdentifier("cloudBackup.finishingDisable")

            Text(
                "Cove couldn't finish deleting your backups from iCloud. Your wallets on this device aren't affected. Some backups may already be deleted, and new changes aren't being backed up."
            )
            .font(.caption)
            .uiTestTruncationProbe()
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            CloudBackupFinishingDisableStageRow(stage: stage)

            CloudBackupActionControl(
                action: .resumeDisable,
                actions: actions,
                perform: { manager.dispatch(action: .resumeDisable) }
            ) {
                ProbedLabel("Try Again", systemImage: "arrow.clockwise")
            }
        }

        CloudBackupStopUsingSection(actions: actions, manager: manager)
    }
}

/// Why the last attempt failed, or that a new one is deleting
private struct CloudBackupFinishingDisableStageRow: View {
    let stage: CloudBackupFinishingDisableStage

    private var failureMessage: String? {
        guard case let .failed(reason) = stage else { return nil }

        return switch reason {
        case .offline:
            "Cove can't reach iCloud right now."
        case .authorizationRequired:
            "Cove can't access iCloud Drive. Turn on iCloud Drive for Cove, then try again."
        case .providerUnavailable:
            "iCloud Drive isn't available right now."
        case .other:
            nil
        }
    }

    var body: some View {
        if let failureMessage {
            Text(failureMessage)
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(Color.statusError)
        }

        if case .resuming = stage {
            HStack {
                ProgressView()
                    .padding(.trailing, 8)
                Text("Deleting your backups from iCloud...")
                    .font(.footnote)
                    .uiTestTruncationProbe()
            }
        }
    }
}

/// Leaves the backups in iCloud and turns Cloud Backup off on this device only
private struct CloudBackupStopUsingSection: View {
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    var body: some View {
        Section {
            CloudBackupActionControl(
                action: .stopUsingOnThisDevice,
                actions: actions,
                role: .destructive,
                perform: { manager.dispatch(action: .stopUsingOnThisDevice) }
            ) {
                ProbedLabel("Stop Using Cloud Backup on This Device", systemImage: "iphone.slash")
            }
        } footer: {
            Text("Your backups may still be in iCloud. You can delete them from iCloud settings.")
                .uiTestTruncationProbe()
        }
    }
}

private struct CloudBackupLoadFailedSection: View {
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    var body: some View {
        Section {
            ProbedLabel("Cove couldn't load Cloud Backup", systemImage: "exclamationmark.icloud")
                .foregroundStyle(Color.statusError)
                .accessibilityIdentifier("cloudBackup.loadFailed")

            Text("Your wallets on this device aren't affected.")
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(.secondary)

            CloudBackupActionControl(
                action: .retryLoad,
                actions: actions,
                perform: { manager.dispatch(action: .retryLoad) }
            ) {
                ProbedLabel("Try Again", systemImage: "arrow.clockwise")
            }
        }
    }
}

private struct CloudBackupCorruptedSection: View {
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    var body: some View {
        Section {
            ProbedLabel("Cove can't read the Cloud Backup settings", systemImage: "exclamationmark.icloud")
                .foregroundStyle(Color.statusError)
                .accessibilityIdentifier("cloudBackup.corrupted")

            Text(
                "Cloud Backup's settings on this device are damaged. Contact support before changing Cloud Backup settings. Resetting turns Cloud Backup off on this device; your wallets on this device aren't affected."
            )
            .font(.caption)
            .uiTestTruncationProbe()
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            CloudBackupActionControl(
                action: .resetOnThisDevice,
                actions: actions,
                role: .destructive,
                perform: { manager.dispatch(action: .resetOnThisDevice) }
            ) {
                ProbedLabel("Reset Cloud Backup on This Device", systemImage: "arrow.counterclockwise")
            }
        }
    }
}
