import SwiftUI

private struct TxRowModel: Identifiable {
    let id = UUID()
    let address: Address
    let amount: String
}

struct SendFlowUtxoCustomAmountSheetView: View {
    @Environment(AppManager.self) private var app
    @Environment(WalletManager.self) private var walletManager
    @Environment(SendFlowManager.self) private var manager
    @Environment(SendFlowPresenter.self) private var presenter
    @Environment(\.dismiss) private var dismiss

    var metadata: WalletMetadata { walletManager.walletMetadata }

    let utxos: [Utxo]

    // private
    @State private var customAmount: Double = 0.0

    var customAmountBinding: Binding<Double> {
        Binding(
            get: { customAmount },
            set: {
                customAmount = $0
                manager.dispatch(.notifyCoinControlAmountChanged($0))
            }
        )
    }

    @ViewBuilder
    private var divider: some View {
        Divider()
            .padding(.vertical, 28)
            .foregroundStyle(.red)
    }

    var maxSendSat: Double {
        Double(Int(manager.rust.maxSendMinusFees()?.asSats() ?? 10000))
    }

    var maxSendBtc: Double {
        manager.rust.maxSendMinusFees()?.asBtc() ?? 0.0001
    }

    var displayAmount: String {
        manager.sendAmountBtc
    }

    var body: some View {
        VStack(spacing: 24) {
            // header
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Sending UTXO Details")
                        .font(.headline.weight(.semibold))

                    Text("Your are sending the following UTXOs to the recipient.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .padding(.top)

                Spacer()

                Button(action: { dismiss() }) {
                    Image(systemName: "xmark")
                        .font(.body.weight(.semibold))
                        .foregroundColor(.primary.opacity(0.8))
                        .padding(10)
                        .background(Circle().fill(Color.secondary.opacity(0.15)))
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
            }

            Divider()
                .padding(.horizontal, -16)

            // content sections
            ScrollView {
                ForEach(utxos) { utxo in
                    UtxoRow(utxo: utxo)
                }
            }

            Spacer()

            VStack {
                HStack {
                    Text("Set Amount")
                    Spacer()
                    Text(displayAmount)
                }

                switch metadata.selectedUnit {
                case .sat:
                    Slider(value: $customAmount, in: 0 ... maxSendSat, step: 100)
                case .btc:
                    Slider(value: $customAmount, in: 0 ... maxSendBtc, step: 0.00000100)
                }
            }
        }
        .padding()
        .background(Color(UIColor.secondarySystemBackground))
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .onChange(of: metadata.selectedUnit, initial: true) { _, new in
            if customAmount != maxSendSat, customAmount != maxSendBtc, customAmount > 0 {
                return
            }

            switch new {
            case .sat: customAmount = maxSendSat
            case .btc: customAmount = maxSendBtc
            }
        }
    }
}

private struct UtxoRow: View {
    @Environment(WalletManager.self) private var wm
    let utxo: Utxo

    var body: some View {
        HStack(spacing: 20) {
            VStack(alignment: .leading, spacing: 4) {
                // Name
                HStack(spacing: 4) {
                    Text(utxo.name)
                        .font(.footnote)
                        .truncationMode(.middle)
                        .lineLimit(1)

                    if utxo.type == .change {
                        Image(systemName: "circlebadge.2")
                            .font(.caption)
                            .foregroundColor(.orange.opacity(0.8))
                    }
                }

                // Address (semi-bold caption)
                HStack {
                    Text(utxo.address.spacedOut())
                        .truncationMode(.middle)
                        .font(.caption2)
                        .fontWeight(.semibold)
                        .lineLimit(1)
                        .foregroundColor(.secondary)
                        .truncationMode(.middle)
                }
            }

            Spacer(minLength: 8)

            VStack(alignment: .trailing, spacing: 4) {
                Text(wm.displayAmount(utxo.amount))
                    .font(.footnote)
                    .fontWeight(.regular)

                Text(utxo.date)
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .padding()
        .background(Color.systemBackground)
        .cornerRadius(10)
        .contextMenu {
            Button(action: {
                UIPasteboard.general.string = utxo.address.toString()
            }) {
                Text("Copy Address")
            }

            Button(action: {
                UIPasteboard.general.string = utxo.outpoint.txidStr()
            }) {
                Text("Copy Transaction ID")
            }
        } preview: {
            UtxoRowPreview(displayAmount: wm.displayAmount, utxo: utxo)
        }
    }
}

#Preview {
    AsyncPreview {
        let wm = WalletManager(preview: "preview_only")
        let ap = AppManager.shared
        let presenter = SendFlowPresenter(app: ap, manager: wm)
        let sendFlowManager = ap.getSendFlowManager(wm, presenter: presenter)
        let utxos = previewNewUtxoList(outputCount: 2, changeCount: 1)

        SendFlowUtxoCustomAmountSheetView(utxos: utxos)
            .environment(wm)
            .environment(ap)
            .environment(presenter)
            .environment(sendFlowManager)
            .onAppear {
                wm.dispatch(action: .updateUnit(.sat))
                sendFlowManager.dispatch(.notifySelectedUnitedChanged(old: .btc, new: .sat))
                sendFlowManager.dispatch(.setCoinControlMode(utxos))
            }
    }
}
