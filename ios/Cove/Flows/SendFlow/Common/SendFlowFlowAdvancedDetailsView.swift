import SwiftUI

private struct TxRowModel: Identifiable {
    let id = UUID()
    let label: String?
    let utxoType: UtxoType?
    let address: Address
    let amount: String
}

struct SendFlowAdvancedDetailsView: View {
    @Environment(AppManager.self) private var app
    @Environment(SendFlowPresenter.self) private var presenter
    @Environment(\.dismiss) private var dismiss

    let manager: WalletManager
    let details: ConfirmDetails

    // private
    @State private var splitOutput: SplitOutput? = nil

    var metadata: WalletMetadata { manager.walletMetadata }

    func fiatAmount(_ amount: Amount) -> String {
        guard let prices = app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        return manager.rust.convertAndDisplayFiat(amount: amount, prices: prices)
    }

    func displayFiatOrBtcAmount(_ amount: Amount) -> String {
        switch metadata.fiatOrBtc {
        case .fiat:
            return "≈ \(fiatAmount(amount))"
        case .btc:
            let units = manager.walletMetadata.selectedUnit == .sat ? "sats" : "btc"
            return "\(manager.amountFmt(amount)) \(units)"
        }
    }

    @ViewBuilder
    private var divider: some View {
        Divider()
            .padding(.vertical, 28)
            .foregroundStyle(.red)
    }

    private func toTxRows(_ addressAndAmount: [AddressAndAmount]) -> [TxRowModel] {
        addressAndAmount.map {
            TxRowModel(
                label: $0.label,
                utxoType: $0.utxoType,
                address: $0.address,
                amount: self.displayFiatOrBtcAmount($0.amount)
            )
        }
    }

    var body: some View {
        VStack(spacing: 24) {
            // header
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Advanced Details")
                        .font(.headline.weight(.semibold))

                    Text("View current transaction breakdown")
                        .font(.footnote)
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
                VStack(spacing: 0) {
                    if let splitOutput {
                        SectionCard(title: "UTXOs Used", rows: toTxRows(details.inputs()))
                        divider

                        if splitOutput.external.isEmpty {
                            SectionCard(title: "Sent To Self", rows: toTxRows(splitOutput.internal))
                            divider
                        } else {
                            SectionCard(
                                title: "Sent To Address", rows: toTxRows(splitOutput.external)
                            )
                            divider
                        }

                        if !splitOutput.external.isEmpty, !splitOutput.internal.isEmpty {
                            SectionCard(
                                title: "UTXO Change", rows: toTxRows(splitOutput.internal)
                            )
                            divider
                        }
                    }

                    // Loading...
                    if splitOutput == nil {
                        SectionCard(title: "UTXO Inputs", rows: toTxRows(details.inputs()))
                        divider

                        SectionCard(title: "UTXO Outputs", rows: toTxRows(details.outputs()))
                        divider
                    }

                    HStack {
                        Text("Fee")
                            .font(.caption.weight(.medium))
                            .foregroundStyle(.secondary.opacity(0.75))

                        Spacer()
                        Text(displayFiatOrBtcAmount(details.feeTotal()))
                            .font(.footnote)
                            .fontWeight(.regular)
                    }
                    .padding(.horizontal, 12)
                }
            }
            .onTapGesture { manager.dispatch(action: .toggleFiatOrBtc) }
        }
        .padding()
        .background(Color(UIColor.secondarySystemBackground))
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .task {
            splitOutput = try? await manager.rust.splitTransactionOutputs(
                outputs: details.outputs())
        }
    }
}

private struct TxRow: View {
    let model: TxRowModel

    var label: String? {
        if let label = model.label { return label }
        if let utxoType = model.utxoType {
            switch utxoType {
            case .output: return "Receive Address"
            case .change: return "Change Address"
            }
        }

        return .none
    }

    var body: some View {
        HStack(alignment: .top) {
            Menu {
                Button("Copy", systemImage: "doc.on.doc") {
                    UIPasteboard.general.string = model.address.unformatted()
                }
            } label: {
                VStack(alignment: .leading, spacing: 6) {
                    if let label {
                        HStack {
                            Text(label)
                                .font(.footnote)
                                .truncationMode(.middle)
                                .lineLimit(1)
                                .fixedSize(horizontal: false, vertical: true)
                                .multilineTextAlignment(.leading)

                            if case .some(.change) = model.utxoType {
                                Image(systemName: "circlebadge.2")
                                    .font(.caption)
                                    .foregroundColor(.orange.opacity(0.8))
                            }
                        }
                    }

                    Text(model.address.spacedOut())
                        .font(.caption2.monospaced())
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .multilineTextAlignment(.leading)
                }
            }
            .foregroundStyle(.primary)

            Spacer(minLength: 18)
            Text(model.amount)
                .font(.footnote)
        }
        .padding(.vertical, 12)
        .padding(.horizontal, 12)
    }
}

private struct SectionCard: View {
    var title: String? = nil
    let rows: [TxRowModel]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let title {
                Text(title)
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary.opacity(0.75))
                    .padding(.leading, 12)
                    .padding(.bottom, 8)
            }

            VStack(spacing: 0) {
                ForEach(rows.indices, id: \.self) { idx in
                    TxRow(model: rows[idx])
                    if idx < rows.count - 1 {
                        Divider()
                            .padding(.leading, 12)
                    }
                }
            }
            .background(
                RoundedRectangle(cornerRadius: 6)
                    .fill(Color(UIColor.systemBackground))
            )
        }
    }
}

#Preview {
    AsyncPreview {
        SendFlowAdvancedDetailsView(
            manager: WalletManager(preview: "preview_only"),
            details: ConfirmDetails.previewNew()
        )
        .environment(AppManager.shared)
        .environment(
            SendFlowPresenter(
                app: AppManager.shared,
                manager: WalletManager(preview: "preview_only")
            )
        )
    }
}
