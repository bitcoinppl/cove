import SwiftUI

struct KeyTeleportStateContent: View {
    @Bindable var manager: KeyTeleportManager
    let route: KeyTeleportRoute
    @Binding var pastedText: String
    @Binding var receiverCode: String
    @Binding var senderPassword: String
    @Binding var mnemonicDisclosure: KeyTeleportMnemonicDisclosure
    @Binding var xprv: String?
    let scan: () -> Void
    let paste: () -> Void
    let revealMnemonicWords: () -> Void
    let finishReview: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            switch route {
            case .receive:
                KeyTeleportReceiveStateContent(
                    manager: manager,
                    senderPassword: $senderPassword,
                    mnemonicDisclosure: $mnemonicDisclosure,
                    xprv: $xprv,
                    scan: scan,
                    revealMnemonicWords: revealMnemonicWords,
                    finishReview: finishReview
                )
            case .send:
                KeyTeleportSendStateContent(
                    manager: manager,
                    pastedText: $pastedText,
                    receiverCode: $receiverCode,
                    scan: scan,
                    paste: paste
                )
            }
        }
        .keyTeleportCard()
    }
}

struct KeyTeleportReceiveStateContent: View {
    @Bindable var manager: KeyTeleportManager
    @Binding var senderPassword: String
    @Binding var mnemonicDisclosure: KeyTeleportMnemonicDisclosure
    @Binding var xprv: String?
    let scan: () -> Void
    let revealMnemonicWords: () -> Void
    let finishReview: () -> Void

    var body: some View {
        switch manager.state {
        case .idle, .receiveReady, .receiveError, .receiveEnterPassword:
            KeyTeleportReceiveSetupStateContent(
                manager: manager,
                senderPassword: $senderPassword,
                scan: scan
            )
        default:
            KeyTeleportReceiveReviewStateContent(
                manager: manager,
                mnemonicDisclosure: $mnemonicDisclosure,
                xprv: $xprv,
                revealMnemonicWords: revealMnemonicWords,
                finishReview: finishReview
            )
        }
    }
}

struct KeyTeleportSendStateContent: View {
    @Environment(AppManager.self) private var app

    @Bindable var manager: KeyTeleportManager
    @Binding var pastedText: String
    @Binding var receiverCode: String
    let scan: () -> Void
    let paste: () -> Void

    var body: some View {
        switch manager.state {
        case .idle:
            KeyTeleportScanPasteSection(pastedText: $pastedText, scan: scan, paste: paste)
        case .sendAwaitReceiver:
            KeyTeleportAwaitReceiverView()
            KeyTeleportScanPasteSection(pastedText: $pastedText, scan: scan, paste: paste)
        case let .sendChooseWallet(state):
            KeyTeleportSendChooseWalletView(state: state, select: selectWallet)
        case let .sendEnterCode(state):
            KeyTeleportReceiverCodeView(
                state: state,
                code: $receiverCode,
                submit: submitReceiverCode
            )
        case let .sendReady(state):
            KeyTeleportSendReadyView(state: state, finish: finish)
        default:
            EmptyView()
        }
    }

    private func selectWallet(_ walletId: WalletId) {
        manager.dispatch(.selectSendWallet(walletId))
    }

    private func submitReceiverCode() {
        manager.dispatch(.enterReceiverCode(receiverCode))
    }

    private func finish() {
        manager.dispatch(.clear)
        app.popRoute()
    }
}
