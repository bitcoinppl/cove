import SwiftUI

struct KeyTeleportScanPasteSection: View {
    @Binding var pastedText: String
    let scan: () -> Void
    let paste: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Button(action: scan) {
                Label("Scan QR", systemImage: "qrcode.viewfinder")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(OnboardingPrimaryButtonStyle())

            TextField(
                "Paste KeyTeleport packet or link",
                text: $pastedText,
                prompt: keyTeleportInputPlaceholder("Paste KeyTeleport packet or link"),
                axis: .vertical
            )
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .lineLimit(3, reservesSpace: true)
            .keyTeleportInputChrome()

            Button(action: paste) {
                Label("Use Pasted Packet", systemImage: "doc.on.clipboard")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }
}

struct KeyTeleportAwaitReceiverView: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Scan Receiver Request")
                .font(.headline)

            Text("Scan or paste the request shown on the receiving device.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
    }
}

struct KeyTeleportSendChooseWalletView: View {
    let state: KeyTeleportSendChooseWallet
    let select: (WalletId) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Wallet")
                .font(.headline)

            ForEach(state.eligibleWallets, id: \.id) { wallet in
                Button {
                    select(wallet.id)
                } label: {
                    HStack {
                        Text(wallet.name)
                        Spacer()
                    }
                }
                .buttonStyle(.bordered)
                .tint(.white)
            }
        }
    }
}

struct KeyTeleportReceiverCodeView: View {
    let state: KeyTeleportSendEnterCode
    @Binding var code: String
    let submit: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Sending from \(state.selectedWallet.name)")
                .font(.headline)

            TextField(
                "Receiver Code",
                text: $code,
                prompt: keyTeleportInputPlaceholder("Receiver Code")
            )
            .keyboardType(.numberPad)
            .keyTeleportInputChrome()

            Button(action: submit) {
                Text("Continue")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(OnboardingPrimaryButtonStyle())
        }
    }
}

struct KeyTeleportSendReadyView: View {
    let state: KeyTeleportSendReady
    let finish: () -> Void

    var body: some View {
        VStack(spacing: 18) {
            Text("Sending \(state.selectedWallet.name)")
                .font(.headline)
                .frame(maxWidth: .infinity, alignment: .leading)

            if let packet = try? state.packet.bbqrPart() {
                KeyTeleportRevealPair(
                    qrHint: "Tap to show QR code",
                    codeHint: "Tap to show password"
                ) {
                    QrCodeView(text: packet)
                        .frame(maxWidth: 280)
                        .frame(maxWidth: .infinity)
                } code: {
                    teleportPassword
                }
            } else {
                Text("Unable to render this sender response.")
                    .foregroundStyle(.red)

                teleportPassword
            }

            Text("Show the QR code to the receiver in person or over video, and send the password through a different channel, like a call or message. Only one is visible at a time — tap the hidden one to reveal it.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            Button("Done", action: finish)
                .buttonStyle(OnboardingPrimaryButtonStyle())
        }
    }

    private var teleportPassword: some View {
        VStack(spacing: 4) {
            Text("Teleport Password")
                .font(.caption)
                .foregroundStyle(.secondary)
            KeyTeleportCodeText(state.password.groupedText())
        }
    }
}
