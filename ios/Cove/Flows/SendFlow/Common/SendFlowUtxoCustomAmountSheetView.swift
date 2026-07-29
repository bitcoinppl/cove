import SwiftUI

struct SendFlowUtxoCustomAmountSheetView: View {
    @Environment(AppManager.self) private var app
    @Environment(WalletManager.self) private var walletManager
    @Environment(SendFlowManager.self) private var manager
    @Environment(\.dismiss) private var dismiss

    var metadata: WalletMetadata {
        walletManager.walletMetadata
    }

    let utxos: [Utxo]

    // private
    @State private var customAmount: Double = 0.0
    @State private var previousAmount: Double = .init(conservativeDustLimitSats)
    @State private var isEditing: Bool = false
    @State private var pinState: PinState = .hard
    private enum PinState { case none, soft, hard }

    @State private var enteringAmount: String? = nil

    @FocusState private var isFocused: Bool

    private var presenter: SendFlowPresenter {
        manager.presenter
    }

    private var smartSnapBinding: Binding<Double> {
        Binding(
            get: { customAmount },
            set: { raw in
                enteringAmount = nil
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
                    manager.debouncedDispatch(
                        .notifyCoinControlAmountChanged(adjusted),
                        for: .milliseconds(200)
                    )
                }

                previousAmount = raw
            }
        )
    }

    private var minSend: Double {
        satToDouble(conservativeDustLimitSats)
    }

    private var step: Double {
        satToDouble(10)
    }

    private var maxSend: Double {
        var amount = manager.rust.maxSendMinusFees() ?? Amount.fromSat(sats: conservativeDustLimitSatsU + 1000)
        if amount.asSats() <= conservativeDustLimitSatsU {
            amount = Amount.fromSat(sats: conservativeDustLimitSatsU + 1000)
        }
        return amountToDouble(amount)
    }

    /// softMaxSend is the next biggest amount below maxSend that can be selected
    /// any amount between softMaxSend and maxSend can NOT be selected, because that would create a dust UTXO
    private var softMaxSend: Double {
        let amount = manager.rust.maxSendMinusFeesAndSmallUtxo() ?? conservativeDustLimitAmount
        return amountToDouble(amount)
    }

    private func displayAmount(_ amount: String? = nil) -> String {
        let amountDouble = amount.flatMap {
            manager.rust.sanitizeBtcEnteringAmount(oldValue: enteringAmount ?? "", newValue: $0)
        }.map { $0.replacingOccurrences(of: ",", with: "") }.flatMap { Double($0) }
        let amount =
            switch (metadata.selectedUnit, amountDouble) {
            case let (.sat, .some(amount)):
                Amount.fromSat(sats: UInt64(amount))
            case let (.btc, .some(amount)):
                Amount.fromSat(sats: UInt64(amount * 100_000_000))
            case (.sat, nil):
                Amount.fromSat(sats: UInt64(customAmount))
            case (.btc, nil):
                Amount.fromSat(sats: UInt64(customAmount * 100_000_000))
            }

        return walletManager.displayAmount(amount, showUnit: false)
    }

    private var displayAmountBinding: Binding<String> {
        Binding(
            get: { displayAmount(enteringAmount) },
            set: {
                enteringAmount = $0
                manager.dispatch(.notifyCoinControlEnteredAmountChanged($0, isFocused))
            }
        )
    }

    private func satToDouble(_ sats: Int) -> Double {
        amountToDouble(Amount.fromSat(sats: UInt64(sats)))
    }

    private func amountToDouble(_ amount: Amount) -> Double {
        switch metadata.selectedUnit {
        case .sat: Double(amount.asSats())
        case .btc: amount.asBtc()
        }
    }

    var selectedUnitSymbol: String {
        switch metadata.selectedUnit {
        case .sat: "SATS"
        case .btc: "BTC"
        }
    }

    var body: some View {
        VStack(spacing: 24) {
            SendFlowUtxoCustomAmountHeader(dismiss: dismiss.callAsFunction)
                .onTapGesture { isFocused = false }

            Divider()
                .padding(.horizontal, -16)

            ScrollView {
                ForEach(utxos) { utxo in
                    UtxoRow(utxo: utxo)
                }
            }
            .onTapGesture { isFocused = false }

            Spacer()

            SendFlowUtxoCustomAmountControls(
                displayAmount: displayAmount(),
                displayAmountBinding: displayAmountBinding,
                selectedUnitSymbol: selectedUnitSymbol,
                sliderValue: smartSnapBinding,
                sliderRange: minSend ... maxSend,
                sliderStep: step,
                isFocused: $isFocused,
                editingChanged: { isEditing = $0 }
            )
        }
        .padding()
        .background(Color(UIColor.secondarySystemBackground))
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .onChange(of: metadata.selectedUnit, initial: true, selectedUnitChanged)
        .onChange(of: isEditing, editingChanged)
        .onChange(of: manager.amount, managerAmountChanged)
        .onChange(of: isFocused, initial: false, focusChanged)
    }

    private func selectedUnitChanged(_: Unit, _: Unit) {
        customAmount = manager.amount.map(amountToDouble) ?? maxSend
    }

    private func editingChanged(_ old: Bool, _ new: Bool) {
        Log.debug("isEditing changed from \(old) -> \(new)")

        if old, !new {
            manager.dispatch(.notifyCoinControlAmountChanged(customAmount))
        }
    }

    private func managerAmountChanged(_: Amount?, _ new: Amount?) {
        guard let newAmount = new, !isEditing else { return }

        switch metadata.selectedUnit {
        case .sat: customAmount = Double(newAmount.asSats())
        case .btc: customAmount = newAmount.asBtc()
        }
    }

    private func focusChanged(_ old: Bool, _ new: Bool) {
        guard old, !new else { return }

        customAmount = manager.amount.map(amountToDouble) ?? maxSend
    }
}

private struct SendFlowUtxoCustomAmountHeader: View {
    let dismiss: () -> Void

    var body: some View {
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

            Button(action: dismiss) {
                Image(systemName: "xmark")
                    .font(.body.weight(.semibold))
                    .foregroundColor(.primary.opacity(0.8))
                    .padding(10)
                    .background(Circle().fill(Color.secondary.opacity(0.15)))
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
        }
    }
}

private struct SendFlowUtxoCustomAmountControls: View {
    let displayAmount: String
    let displayAmountBinding: Binding<String>
    let selectedUnitSymbol: String
    let sliderValue: Binding<Double>
    let sliderRange: ClosedRange<Double>
    let sliderStep: Double
    let isFocused: FocusState<Bool>.Binding
    let editingChanged: (Bool) -> Void

    var body: some View {
        VStack {
            HStack {
                Text("Set Amount")
                Spacer()
                TextField(displayAmount, text: displayAmountBinding)
                    .keyboardType(.decimalPad)
                    .multilineTextAlignment(.trailing)
                    .focused(isFocused)

                Text(selectedUnitSymbol)
            }
            .font(.subheadline)
            .fontWeight(.semibold)

            HStack {
                Text("Use the slider to set the amount.")
                Spacer()
            }
            .foregroundStyle(.secondary)
            .font(.caption2)

            Slider(
                value: sliderValue,
                in: sliderRange,
                step: sliderStep,
                onEditingChanged: editingChanged
            )
            .accessibilityLabel("Send amount slider")
            .accessibilityValue("\(displayAmount) \(selectedUnitSymbol)")
        }
    }
}

private struct UtxoRow: View {
    @Environment(WalletManager.self) private var wm
    let utxo: Utxo

    var body: some View {
        HStack(spacing: 20) {
            UtxoRowIdentity(utxo: utxo)

            Spacer(minLength: 8)

            VStack(alignment: .trailing, spacing: 4) {
                Text(wm.displayAmount(utxo.amount))
                    .font(.footnote)
                    .fontWeight(.regular)

                Text(utxo.date())
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .padding()
        .background(Color.systemBackground)
        .cornerRadius(10)
        .contextMenu {
            Button(action: {
                UIPasteboard.general.string = utxo.address.unformatted()
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

private struct UtxoRowIdentity: View {
    let utxo: Utxo

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 4) {
                Text(utxo.name())
                    .font(.footnote)
                    .truncationMode(.middle)
                    .lineLimit(1)

                if utxo.type == .change {
                    Image(systemName: "circlebadge.2")
                        .font(.caption)
                        .foregroundColor(.orange.opacity(0.8))
                }
            }

            HStack {
                Text(utxo.address.spacedOut())
                    .font(.caption2)
                    .fontWeight(.semibold)
                    .lineLimit(1)
                    .foregroundColor(.secondary)
                    .truncationMode(.middle)
            }
        }
    }
}

#Preview {
    AsyncPreview {
        let wm = WalletManager(preview: "preview_only")
        let ap = AppManager.shared
        let presenter = SendFlowPresenter(app: ap, manager: wm)
        let utxos = previewNewUtxoList(outputCount: 2, changeCount: 1)

        if let sendFlowManager = try? ap.ensureSendFlowManager(wm, presenter: presenter) {
            SendFlowUtxoCustomAmountSheetView(utxos: utxos)
                .environment(wm)
                .environment(ap)
                .environment(presenter)
                .environment(sendFlowManager)
        }
    }
}
