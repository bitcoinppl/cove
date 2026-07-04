import SwiftUI

extension MainSettingsAlertState: TaggedAlertPresentable {
    func alert(context: MainSettingsPresentationContext) -> AnyAlertBuilder {
        switch self {
        case let .unverifiedWallets(walletId):
            AlertBuilder(
                title: "Can't Enable Wipe Data PIN",
                message: """
                You have wallets that have not been backed up. Please back up your wallets before enabling the Wipe Data PIN.\
                If you wipe the data without having a back up of your wallet, you will lose the bitcoin in that wallet.
                """,
                actions: {
                    Button("Go To Wallet") {
                        try? context.app.selectWalletOrThrow(walletId)
                    }

                    Button("Cancel", role: .cancel) { context.dismissAlert() }
                }
            ).eraseToAny()

        case .confirmEnableWipeMePin:
            AlertBuilder(
                title: "Are you sure?",
                message:
                """

                Enabling the Wipe Data PIN will let you chose a PIN that if entered will wipe all Cove wallet data on this device.

                If you wipe the data without having a back up of your wallet, you will lose the bitcoin in that wallet.\u{20}

                Please make sure you have a backup of your wallet before enabling this.
                """,
                actions: {
                    Button("Yes, Enable Wipe Data PIN") {
                        context.dismissAlert()
                        context.presentSheet(.enableWipeDataPin)
                    }
                    Button("Cancel", role: .cancel) { context.dismissAlert() }
                }
            ).eraseToAny()

        case .confirmDecoyPin:
            AlertBuilder(
                title: "Are you sure?",
                message:
                """

                Enabling Decoy PIN will let you chose a PIN that if entered, will show you a different set of wallets.

                These wallets will only be accessible by entering the decoy PIN instead of your regular PIN.

                To access your regular wallets, you will have to close the app, start it again and enter your regular PIN.
                """,
                actions: {
                    Button("Yes, Enable Decoy PIN") {
                        context.dismissAlert()
                        context.presentSheet(.enableDecoyPin)
                    }
                    Button("Cancel", role: .cancel) { context.dismissAlert() }
                }
            ).eraseToAny()

        case .notePinRequired:
            AlertBuilder(
                title: "PIN is required",
                message: "Setting a PIN is required to have a wipe data PIN",
                actions: { Button("OK") { context.dismissAlert() } }
            ).eraseToAny()

        case let .noteFaceIdDisabling(nextAlertState):
            AlertBuilder(
                title: "Disable FaceID Unlock?",
                message: """

                Enabling this trick PIN will disable FaceID unlock for Cove.\u{20}

                Going forward, you will have to use your PIN to unlock Cove.
                """,
                actions: {
                    Button("Disable FaceID", role: .destructive) {
                        context.auth.dispatch(action: .disableBiometric)
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.350) {
                            context.presentAlert(nextAlertState)
                        }
                    }
                    Button("Cancel", role: .cancel) { context.dismissAlert() }
                }
            ).eraseToAny()

        case .noteNoFaceIdWhenTrickPins:
            AlertBuilder(
                title: "Can't do that",
                message: """

                You can't have Decoy PIN & Wipe Data Pin enabled and FaceID active at the same time.

                Do you wan't to disable both of these trick PINs and enable FaceID?
                """,
                actions: {
                    Button("Cancel", role: .cancel) { context.dismissAlert() }
                    Button("Yes, Disable trick PINs", role: .destructive) {
                        context.presentSheet(.removeAllTrickPins)
                    }
                }
            ).eraseToAny()

        case .noteNoFaceIdWhenWipeMePin:
            AlertBuilder(
                title: "Can't do that",
                message: "You can't have both Wipe Data PIN and FaceID active at the same time",
                actions: {
                    Button("Cancel", role: .cancel) { context.dismissAlert() }
                    Button("Disable Wipe Data PIN", role: .destructive) {
                        if !context.auth.isDecoyPinEnabled {
                            let nextSheetState = TaggedItem(MainSettingsSheetState.enableBiometric)
                            context.presentSheet(.removeWipeDataPin(nextSheetState))
                        } else {
                            context.presentSheet(.removeWipeDataPin(.none))
                        }
                    }
                }
            ).eraseToAny()

        case .noteNoFaceIdWhenDecoyPin:
            AlertBuilder(
                title: "Can't do that",
                message: "You can't have both Decoy PIN and FaceID active at the same time",
                actions: {
                    Button("Cancel", role: .cancel) { context.dismissAlert() }
                    Button("Disable Decoy Pin", role: .destructive) {
                        if !context.auth.isWipeDataPinEnabled {
                            let nextSheetState = TaggedItem(MainSettingsSheetState.enableBiometric)
                            context.presentSheet(.removeDecoyPin(nextSheetState))
                        } else {
                            context.presentSheet(.removeDecoyPin(.none))
                        }
                    }
                }
            ).eraseToAny()

        case .confirmBetaImportExport:
            AlertBuilder(
                title: "Experimental Feature",
                message: "This is a very experimental feature. Use with caution. This is mostly used by developers for testing purposes.",
                actions: {
                    Button("Accept") {
                        try? Database().globalFlag().set(key: .betaImportExportEnabled, value: true)
                        context.isBetaImportExportEnabled.wrappedValue = true
                        context.dismissAlert()
                    }
                    Button("Cancel", role: .cancel) { context.dismissAlert() }
                }
            ).eraseToAny()

        case let .extraSetPinError(error):
            AlertBuilder(
                title: "Something went wrong!",
                message: error,
                actions: { Button("OK") { context.dismissAlert() } }
            )
            .eraseToAny()
        }
    }
}
