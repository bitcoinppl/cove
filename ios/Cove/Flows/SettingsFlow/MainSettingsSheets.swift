import SwiftUI

extension MainSettingsSheetState: TaggedSheetPresentable {
    func sheet(context: MainSettingsPresentationContext) -> AnyView {
        AnyView(MainSettingsSheetContent(sheet: self, context: context))
    }
}

private struct MainSettingsSheetContent: View {
    private let content: AnyView

    init(sheet: MainSettingsSheetState, context: MainSettingsPresentationContext) {
        content = MainSettingsSheetFactory.content(for: sheet, context: context)
    }

    var body: some View {
        content
    }
}

@MainActor
private enum MainSettingsSheetFactory {
    static func content(
        for sheet: MainSettingsSheetState,
        context: MainSettingsPresentationContext
    ) -> AnyView {
        switch sheet {
        case .enableAuth:
            enableAuth(context: context)
        case .newPin:
            newPin(context: context)
        case .removePin:
            removePin(context: context)
        case let .removeWipeDataPin(nextSheet):
            removeWipeDataPin(nextSheet: nextSheet, context: context)
        case let .removeDecoyPin(nextSheet):
            removeDecoyPin(nextSheet: nextSheet, context: context)
        case .removeAllTrickPins:
            removeAllTrickPins(context: context)
        case .changePin:
            changePin(context: context)
        case .disableBiometric:
            disableBiometric(context: context)
        case .enableBiometric:
            enableBiometric(context: context)
        default:
            secondaryContent(for: sheet, context: context)
        }
    }

    private static func secondaryContent(
        for sheet: MainSettingsSheetState,
        context: MainSettingsPresentationContext
    ) -> AnyView {
        switch sheet {
        case .enableWipeDataPin:
            enableWipeDataPin(context: context)
        case .enableDecoyPin:
            enableDecoyPin(context: context)
        case .backupExportAuth:
            backupExportAuth(context: context)
        case .backupExport:
            backupExport(context: context)
        case .backupImport:
            backupImport(context: context)
        case .backupVerify:
            backupVerify(context: context)
        case .cloudBackupOnboarding:
            cloudBackupOnboarding(context: context)
        default:
            preconditionFailure("Primary settings sheet routed to the secondary factory")
        }
    }

    private static func enableAuth(context: MainSettingsPresentationContext) -> AnyView {
        AnyView(MainSettingsEnableAuthSheet(context: context))
    }

    private static func newPin(context: MainSettingsPresentationContext) -> AnyView {
        AnyView(NewPinView(
            onComplete: context.setPin,
            backAction: context.dismissSheet
        ))
    }

    private static func removePin(context: MainSettingsPresentationContext) -> AnyView {
        let auth = context.auth

        return AnyView(NumberPadPinView(
            title: "Enter Current PIN",
            isPinCorrect: { pin in
                if auth.isInDecoyMode() { return auth.checkDecoyPin(pin) }
                return auth.checkPin(pin)
            },
            showPin: false,
            backAction: context.dismissSheet,
            onUnlock: { _ in
                if auth.isInDecoyMode() {
                    context.dismissSheet()
                    context.isPinEnabled.wrappedValue = false
                    return
                }

                auth.dispatch(action: .disablePin)
                auth.dispatch(action: .disableWipeDataPin)
                context.dismissSheet()
            }
        ))
    }

    private static func removeWipeDataPin(
        nextSheet: TaggedItem<MainSettingsSheetState>?,
        context: MainSettingsPresentationContext
    ) -> AnyView {
        let auth = context.auth

        return AnyView(NumberPadPinView(
            title: "Enter Current PIN",
            isPinCorrect: auth.checkPin,
            showPin: false,
            backAction: context.dismissSheet,
            onUnlock: { _ in
                if auth.isInDecoyMode() { return }
                auth.dispatch(action: .disableWipeDataPin)
                context.setSheet(nextSheet)
            }
        ))
    }

    private static func removeDecoyPin(
        nextSheet: TaggedItem<MainSettingsSheetState>?,
        context: MainSettingsPresentationContext
    ) -> AnyView {
        let auth = context.auth

        return AnyView(NumberPadPinView(
            title: "Enter Current PIN",
            isPinCorrect: auth.checkPin,
            showPin: false,
            backAction: context.dismissSheet,
            onUnlock: { _ in
                auth.dispatch(action: .disableDecoyPin)
                context.setSheet(nextSheet)
            }
        ))
    }

    private static func removeAllTrickPins(
        context: MainSettingsPresentationContext
    ) -> AnyView {
        let auth = context.auth

        return AnyView(NumberPadPinView(
            title: "Enter Current PIN",
            isPinCorrect: auth.checkPin,
            showPin: false,
            backAction: context.dismissSheet,
            onUnlock: { _ in
                auth.dispatch(action: .disableDecoyPin)
                auth.dispatch(action: .disableWipeDataPin)
                context.presentSheet(.enableBiometric)
            }
        ))
    }

    private static func changePin(context: MainSettingsPresentationContext) -> AnyView {
        let auth = context.auth

        return AnyView(ChangePinView(
            isPinCorrect: { pin in
                if auth.isInDecoyMode() { return auth.checkDecoyPin(pin) }
                return auth.checkPin(pin)
            },
            backAction: context.dismissSheet,
            onComplete: { pin in
                if auth.isInDecoyMode() {
                    context.dismissSheet()
                    return
                }

                context.dismissSheet()
                if auth.checkWipeDataPin(pin) {
                    context.presentAlert(
                        .extraSetPinError(
                            "Can't update PIN because its the same as your wipe data PIN"
                        )
                    )
                    return
                }

                context.setPin(pin)
            }
        ))
    }

    private static func disableBiometric(
        context: MainSettingsPresentationContext
    ) -> AnyView {
        let auth = context.auth

        return AnyView(LockView(
            lockType: auth.type,
            isPinCorrect: auth.checkPin,
            onUnlock: { _ in
                auth.dispatch(action: .disableBiometric)
                context.dismissSheet()
            },
            backAction: context.dismissSheet,
            content: EmptyView.init
        ))
    }

    private static func enableBiometric(
        context: MainSettingsPresentationContext
    ) -> AnyView {
        let auth = context.auth

        return AnyView(LockView(
            lockType: .biometric,
            isPinCorrect: { _ in true },
            onUnlock: { _ in
                auth.dispatch(action: .enableBiometric)
                context.dismissSheet()
            },
            backAction: context.dismissSheet,
            content: EmptyView.init
        ))
    }

    private static func enableWipeDataPin(
        context: MainSettingsPresentationContext
    ) -> AnyView {
        AnyView(WipeDataPinView(
            onComplete: context.setWipeDataPin,
            backAction: context.dismissSheet
        ))
    }

    private static func enableDecoyPin(
        context: MainSettingsPresentationContext
    ) -> AnyView {
        AnyView(DecoyPinView(
            onComplete: context.setDecoyPin,
            backAction: context.dismissSheet
        ))
    }

    private static func backupExportAuth(
        context: MainSettingsPresentationContext
    ) -> AnyView {
        let auth = context.auth

        return AnyView(LockView(
            lockType: auth.type,
            isPinCorrect: { pin in
                if auth.isInDecoyMode() { return auth.checkDecoyPin(pin) }
                return auth.checkPin(pin)
            },
            onUnlock: { _ in
                if auth.isInDecoyMode() {
                    context.dismissSheet()
                    return
                }

                context.presentSheet(.backupExport)
            },
            backAction: context.dismissSheet,
            content: EmptyView.init
        ))
    }

    private static func backupExport(context: MainSettingsPresentationContext) -> AnyView {
        AnyView(MainSettingsCancellableNavigationSheet(
            title: "Export Backup",
            dismiss: context.dismissSheet
        ) {
            BackupExportView()
        })
    }

    private static func backupImport(context: MainSettingsPresentationContext) -> AnyView {
        AnyView(MainSettingsBackupImportSheet(dismiss: context.dismissSheet))
    }

    private static func backupVerify(context: MainSettingsPresentationContext) -> AnyView {
        AnyView(MainSettingsCancellableNavigationSheet(
            title: "Verify Backup",
            dismiss: context.dismissSheet
        ) {
            BackupVerifyView()
        })
    }

    private static func cloudBackupOnboarding(
        context: MainSettingsPresentationContext
    ) -> AnyView {
        AnyView(MainSettingsCloudBackupOnboardingSheet(context: context))
    }
}

private struct MainSettingsEnableAuthSheet: View {
    let context: MainSettingsPresentationContext

    var body: some View {
        if context.canUseBiometrics() {
            LockView(
                lockType: .biometric,
                isPinCorrect: { _ in true },
                onUnlock: unlock,
                backAction: context.dismissSheet,
                content: EmptyView.init
            )
        } else {
            NewPinView(
                onComplete: context.setPin,
                backAction: context.dismissSheet
            )
        }
    }

    private func unlock(with pin: String) {
        guard !context.auth.isInDecoyMode() else { return }

        context.auth.dispatch(action: .enableBiometric)
        if !pin.isEmpty {
            context.auth.dispatch(action: .setPin(pin))
        }

        context.dismissSheet()
    }
}

private struct MainSettingsCancellableNavigationSheet<Content: View>: View {
    let title: String
    let dismiss: () -> Void
    let isCancellationDisabled: Bool
    @ViewBuilder let content: Content

    init(
        title: String,
        dismiss: @escaping () -> Void,
        isCancellationDisabled: Bool = false,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.dismiss = dismiss
        self.isCancellationDisabled = isCancellationDisabled
        self.content = content()
    }

    var body: some View {
        NavigationStack {
            content
                .navigationTitle(title)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel", action: dismiss)
                            .disabled(isCancellationDisabled)
                    }
                }
        }
        .interactiveDismissDisabled(isCancellationDisabled)
    }
}

private struct MainSettingsBackupImportSheet: View {
    let dismiss: () -> Void

    @State private var isImporting = false

    var body: some View {
        MainSettingsCancellableNavigationSheet(
            title: "Import Backup",
            dismiss: dismiss,
            isCancellationDisabled: isImporting
        ) {
            BackupImportView(isImporting: $isImporting)
        }
    }
}

private struct MainSettingsCloudBackupOnboardingSheet: View {
    let context: MainSettingsPresentationContext

    var body: some View {
        SettingsCloudBackupEnableSheet(
            onComplete: complete,
            onDismiss: context.dismissSheet
        )
    }

    private func complete() {
        context.dismissSheet()
        DispatchQueue.main.async {
            guard !context.app.currentRoute.isEqual(routeToCheck: .settings(.cloudBackup)) else {
                return
            }

            context.app.pushRoute(.settings(.cloudBackup))
        }
    }
}
