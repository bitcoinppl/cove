import SwiftUI

struct KeyTeleportReceiveSetupStateContent: View {
    @Bindable var manager: KeyTeleportManager
    @Binding var senderPassword: String
    let scan: () -> Void

    var body: some View {
        switch manager.state {
        case .idle:
            KeyTeleportLoadingView()
        case let .receiveReady(state):
            KeyTeleportReceiveReadyView(state: state, scan: scan)
        case .receiveError:
            KeyTeleportReceiveErrorView(retry: retry)
        case .receiveEnterPassword:
            KeyTeleportPasswordEntryView(password: $senderPassword, submit: submitPassword)
        default:
            EmptyView()
        }
    }

    private func retry() {
        manager.dispatch(.startReceive)
    }

    private func submitPassword() {
        manager.dispatch(.enterSenderPassword(senderPassword))
    }
}

struct KeyTeleportReceiveErrorView: View {
    let retry: () -> Void

    var body: some View {
        VStack(spacing: 16) {
            Text("Cove couldn’t prepare a receive request.")
                .font(.headline)

            Button("Try Again", action: retry)
                .buttonStyle(OnboardingPrimaryButtonStyle())
        }
    }
}

struct KeyTeleportReceiveReviewStateContent: View {
    @Bindable var manager: KeyTeleportManager
    @Binding var mnemonicDisclosure: KeyTeleportMnemonicDisclosure
    @Binding var xprv: String?
    let revealMnemonicWords: () -> Void
    let finishReview: () -> Void

    var body: some View {
        switch manager.state {
        case let .receiveMnemonicReview(review):
            KeyTeleportMnemonicReviewView(
                review: review,
                disclosure: mnemonicDisclosure,
                reveal: revealMnemonicWords,
                importWords: importMnemonic,
                finish: finishMnemonicReview
            )
        case let .receiveXprvReview(review):
            KeyTeleportXprvReviewView(
                review: review,
                xprv: $xprv,
                reveal: revealXprv,
                hide: hideXprv,
                importWallet: importXprv,
                finish: finishXprvReview
            )
        case let .receiveMessageReview(review):
            KeyTeleportMessageReviewView(review: review, finish: finishReview)
        case let .receiveImportedWallet(wallet):
            KeyTeleportImportedWalletStateView(
                manager: manager,
                wallet: wallet,
                presentation: .imported
            )
        case let .receiveAlreadyImportedWallet(wallet):
            KeyTeleportImportedWalletStateView(
                manager: manager,
                wallet: wallet,
                presentation: .alreadyImported
            )
        default:
            EmptyView()
        }
    }

    private func importMnemonic() {
        mnemonicDisclosure = .hidden
        manager.dispatch(.importReceivedWallet)
    }

    private func finishMnemonicReview() {
        mnemonicDisclosure = .hidden
        finishReview()
    }

    private func revealXprv() {
        xprv = manager.revealXprv()
    }

    private func hideXprv() {
        xprv = nil
        manager.dispatch(.hideXprv)
    }

    private func importXprv() {
        xprv = nil
        manager.dispatch(.importReceivedWallet)
    }

    private func finishXprvReview() {
        xprv = nil
        finishReview()
    }
}

enum KeyTeleportImportedWalletPresentation {
    case imported
    case alreadyImported
}

struct KeyTeleportImportedWalletStateView: View {
    @Environment(AppManager.self) private var app

    @Bindable var manager: KeyTeleportManager
    let wallet: WalletMetadata
    let presentation: KeyTeleportImportedWalletPresentation

    var body: some View {
        switch presentation {
        case .imported:
            KeyTeleportImportedWalletView(wallet: wallet, finish: openWallet)
        case .alreadyImported:
            KeyTeleportImportedWalletView(
                wallet: wallet,
                title: "Wallet already imported",
                message: "\(wallet.name) is already available in Cove.",
                buttonTitle: "Open Wallet",
                finish: openWallet
            )
        }
    }

    private func openWallet() {
        manager.dispatch(.clear)
        app.selectWallet(wallet.id)
    }
}

struct KeyTeleportReceiveReadyView: View {
    let state: KeyTeleportReceiveState
    let scan: () -> Void

    var body: some View {
        VStack(spacing: 18) {
            if let packet = try? state.packet.bbqrPart() {
                KeyTeleportRevealPair(
                    qrHint: "Tap to show QR code",
                    codeHint: "Tap to show receiver code"
                ) {
                    QrCodeView(text: packet)
                        .frame(maxWidth: 280)
                        .frame(maxWidth: .infinity)
                } code: {
                    receiverCode
                }
            } else {
                Text("Unable to render this receive request.")
                    .foregroundStyle(.red)

                receiverCode
            }

            Text(
                """
                Have the sending wallet scan the QR code, then send the receiver code through a different channel, such as a call or message.

                If the sending wallet cannot scan this screen, tap Share and open the link on another device. The link shows the same QR code.
                """
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            Button(action: scan) {
                Label("Scan Sender Response", systemImage: "qrcode.viewfinder")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(OnboardingPrimaryButtonStyle())
        }
    }

    private var receiverCode: some View {
        VStack(spacing: 4) {
            Text("Receiver Code")
                .font(.caption)
                .foregroundStyle(.secondary)
            KeyTeleportCodeText(state.groupedNumericCode)
        }
        .frame(maxWidth: .infinity)
    }
}

struct KeyTeleportPasswordEntryView: View {
    @Binding var password: String
    let submit: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            KeyTeleportSecureInput(text: $password, submit: submit)

            Button(action: submit) {
                Text("Unlock")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(OnboardingPrimaryButtonStyle())
        }
    }
}

struct KeyTeleportImportedWalletView: View {
    let wallet: WalletMetadata
    var title = "Wallet imported"
    var message: String?
    var buttonTitle = "Done"
    let finish: () -> Void

    var body: some View {
        VStack(spacing: 18) {
            OnboardingStatusHero(
                systemImage: "checkmark",
                tint: .green,
                fillColor: .green.opacity(0.16),
                iconSize: 22,
                innerBadgeSize: 58
            )

            VStack(spacing: 8) {
                Text(title)
                    .font(OnboardingRecoveryTypography.compactTitle)

                Text(message ?? "\(wallet.name) is ready to use in Cove.")
                    .font(.subheadline)
                    .foregroundStyle(.coveLightGray.opacity(0.74))
                    .multilineTextAlignment(.center)
            }

            Button(buttonTitle, action: finish)
                .buttonStyle(OnboardingPrimaryButtonStyle())
        }
        .frame(maxWidth: .infinity)
    }
}
