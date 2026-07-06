import SwiftUI

extension MainSettingsSheetState: TaggedSheetPresentable {
    func sheet(context: MainSettingsPresentationContext) -> AnyView {
        AnyView(MainSettingsSheetContent(sheet: self, context: context))
    }
}

private struct MainSettingsSheetContent: View {
    let sheet: MainSettingsSheetState
    let context: MainSettingsPresentationContext

    private var auth: AuthManager {
        context.auth
    }

    var body: some View {
        switch sheet {
        case .enableAuth:
            if context.canUseBiometrics() {
                LockView(
                    lockType: .biometric,
                    isPinCorrect: { _ in true },
                    onUnlock: { pin in
                        if auth.isInDecoyMode() { return }
                        auth.dispatch(action: .enableBiometric)
                        if !pin.isEmpty { auth.dispatch(action: .setPin(pin)) }

                        context.dismissSheet()
                    },
                    backAction: { context.dismissSheet() },
                    content: { EmptyView() }
                )
            } else {
                NewPinView(onComplete: context.setPin, backAction: { context.dismissSheet() })
            }

        case .newPin:
            NewPinView(onComplete: context.setPin, backAction: { context.dismissSheet() })

        case .removePin:
            NumberPadPinView(
                title: "Enter Current PIN",
                isPinCorrect: { pin in
                    if auth.isInDecoyMode() { return auth.checkDecoyPin(pin) }
                    return auth.checkPin(pin)
                },

                showPin: false,
                backAction: { context.dismissSheet() },
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
            )

        case let .removeWipeDataPin(nextSheet):
            NumberPadPinView(
                title: "Enter Current PIN",
                isPinCorrect: auth.checkPin,
                showPin: false,
                backAction: { context.dismissSheet() },
                onUnlock: { _ in
                    if auth.isInDecoyMode() { return }
                    auth.dispatch(action: .disableWipeDataPin)
                    context.setSheet(nextSheet)
                }
            )

        case let .removeDecoyPin(nextState):
            NumberPadPinView(
                title: "Enter Current PIN",
                isPinCorrect: auth.checkPin,
                showPin: false,
                backAction: { context.dismissSheet() },
                onUnlock: { _ in
                    auth.dispatch(action: .disableDecoyPin)
                    context.setSheet(nextState)
                }
            )

        case .removeAllTrickPins:
            NumberPadPinView(
                title: "Enter Current PIN",
                isPinCorrect: auth.checkPin,
                showPin: false,
                backAction: { context.dismissSheet() },
                onUnlock: { _ in
                    auth.dispatch(action: .disableDecoyPin)
                    auth.dispatch(action: .disableWipeDataPin)
                    context.presentSheet(.enableBiometric)
                }
            )

        case .changePin:
            ChangePinView(
                isPinCorrect: { pin in
                    if auth.isInDecoyMode() { return auth.checkDecoyPin(pin) }
                    return auth.checkPin(pin)
                },
                backAction: { context.dismissSheet() },
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
            )

        case .disableBiometric:
            LockView(
                lockType: auth.type,
                isPinCorrect: auth.checkPin,
                onUnlock: { _ in
                    auth.dispatch(action: .disableBiometric)
                    context.dismissSheet()
                },
                backAction: { context.dismissSheet() },
                content: { EmptyView() }
            )

        case .enableBiometric:
            LockView(
                lockType: .biometric,
                isPinCorrect: { _ in true },
                onUnlock: { _ in
                    auth.dispatch(action: .enableBiometric)
                    context.dismissSheet()
                },
                backAction: { context.dismissSheet() },
                content: { EmptyView() }
            )

        case .enableWipeDataPin:
            WipeDataPinView(
                onComplete: context.setWipeDataPin,
                backAction: {
                    context.dismissSheet()
                }
            )

        case .enableDecoyPin:
            DecoyPinView(
                onComplete: context.setDecoyPin,
                backAction: {
                    context.dismissSheet()
                }
            )

        case .backupExportAuth:
            LockView(
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
                backAction: { context.dismissSheet() },
                content: { EmptyView() }
            )

        case .backupExport:
            NavigationStack {
                BackupExportView()
                    .navigationTitle("Export Backup")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { context.dismissSheet() }
                        }
                    }
            }

        case .backupImport:
            NavigationStack {
                BackupImportView()
                    .navigationTitle("Import Backup")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { context.dismissSheet() }
                        }
                    }
            }

        case .backupVerify:
            NavigationStack {
                BackupVerifyView()
                    .navigationTitle("Verify Backup")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { context.dismissSheet() }
                        }
                    }
            }

        case .cloudBackupOnboarding:
            SettingsCloudBackupEnableSheet(
                onComplete: {
                    context.dismissSheet()
                    DispatchQueue.main.async {
                        guard !context.app.currentRoute.isEqual(routeToCheck: .settings(.cloudBackup)) else {
                            return
                        }

                        context.app.pushRoute(.settings(.cloudBackup))
                    }
                },
                onDismiss: {
                    context.dismissSheet()
                }
            )
        }
    }
}
