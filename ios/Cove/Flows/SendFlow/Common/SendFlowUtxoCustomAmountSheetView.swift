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

    @ViewBuilder
    private var divider: some View {
        Divider()
            .padding(.vertical, 28)
            .foregroundStyle(.red)
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
                // TODO!!!
            }
        }
        .padding()
        .background(Color(UIColor.secondarySystemBackground))
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }
}

#Preview {
    AsyncPreview {
        SendFlowUtxoCustomAmountSheetView(
            manager: WalletManager(preview: "preview_only"),
            details: ConfirmDetails.previewNew()
        )
        .environment(AppManager.shared)
        .environment(
            SendFlowPresenter(
                app: AppManager.shared,
                manager: WalletManager(preview: "preview_only"),
            )
        )
    }
}
