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
    @State private var previousAmount: Double = 10000
    @State private var isEditing: Bool = false
    @State private var pinState: PinState = .hard
    private enum PinState { case none, soft, hard }

    private var smartSnapBinding: Binding<Double> {
        Binding(
            get: { customAmount },
            set: { raw in
                let goingUp = raw > previousAmount
                let goingDown = raw < previousAmount
                var adjusted = raw

                switch pinState {
                case .hard:
                    if goingDown {
                        pinState = .soft
                        adjusted = softMaxSend
                    } else {
                        // hold at pin
                        adjusted = maxSend
                    }

                case .soft:
                    // crossing upward → snap to hard
                    if goingUp {
                        pinState = .hard
                        adjusted = maxSend
                    } else if raw < softMaxSend - step {
                        // pulled a full step below band → release pin
                        pinState = .none
                        adjusted = raw
                    } else {
                        // hold at pin
                        adjusted = softMaxSend
                    }

                case .none:
                    if raw >= softMaxSend {
                        pinState = goingUp ? .hard : .soft
                        adjusted = goingUp ? maxSend : softMaxSend
                    }
                }

                // update model only on real change
                if customAmount != adjusted {
                    customAmount = adjusted
                    manager.deboucedDispatch(
                        .notifyCoinControlAmountChanged(adjusted),
                        for: .milliseconds(200)
                    )
                }

                previousAmount = raw
            }
        )
    }

    @ViewBuilder
    private var divider: some View {
        Divider()
            .padding(.vertical, 28)
            .foregroundStyle(.red)
    }

    var minSend: Double {
        satToDouble(10000)
    }

    var step: Double {
        satToDouble(10)
    }

    var maxSend: Double {
        let amount = manager.rust.maxSendMinusFees() ?? Amount.fromSat(sats: 10000)
        return amountToDouble(amount)
    }

    // softMaxSend is the next biggest amount below maxSend that can be selected
    // any amount between softMaxSend and maxSend can NOT be selected, because that would create a dust UTXO
    var softMaxSend: Double {
        let amount = manager.rust.maxSendMinusFeesAndSmallUtxo() ?? Amount.fromSat(sats: 10000)
        return amountToDouble(amount)
    }

    var displayAmount: String {
        let amountSats =
            switch metadata.selectedUnit {
            case .sat: customAmount
            case .btc: customAmount * 100_000_000
            }

        let amount = Amount.fromSat(sats: UInt64(amountSats))
        return walletManager.displayAmount(amount)
    }

    func satToDouble(_ sats: Int) -> Double {
        amountToDouble(Amount.fromSat(sats: UInt64(sats)))
    }

    func amountToDouble(_ amount: Amount) -> Double {
        switch metadata.selectedUnit {
        case .sat: Double(amount.asSats())
        case .btc: amount.asBtc()
        }
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

                Slider(
                    value: smartSnapBinding,
                    in: minSend ... maxSend,
                    step: step,
                    onEditingChanged: { isEditing = $0 }
                )
            }
        }
        .padding()
        .background(Color(UIColor.secondarySystemBackground))
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .onChange(of: metadata.selectedUnit, initial: true) { _, _ in
            // if the unit changes just default to the max send
            if customAmount != maxSend, customAmount > 0 { return }
            customAmount = maxSend
        }
        .onChange(of: isEditing) { old, new in
            Log.debug("isEditing changed from \(old) -> \(new)")

            // stopped editing dispatch the amount
            if old == true, new == false {
                manager.dispatch(.notifyCoinControlAmountChanged(customAmount))
            }
        }
        .onChange(of: manager.amount) { _, new in
            guard let newAmount = new else { return }
            if isEditing { return }

            switch metadata.selectedUnit {
            case .sat: customAmount = Double(newAmount.asSats())
            case .btc: customAmount = newAmount.asBtc()
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
