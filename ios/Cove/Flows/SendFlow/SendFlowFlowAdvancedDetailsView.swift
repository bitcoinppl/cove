import SwiftUI

struct TxRowModel: Identifiable {
    let id = UUID()
    let address: String
    let amountBTC: String
}

struct SendFlowAdvancedDetailsView: View {
    @Environment(\.dismiss) private var dismiss

    let inputs: [TxRowModel]
    let sentTo: TxRowModel
    let change: [TxRowModel]
    let fee: String

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
                    Text("Advanced Details")
                        .font(.headline.weight(.semibold))

                    Text("View current transaction breakdown")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
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
                    SectionCard(title: "UTXOs Used", rows: inputs)
                    divider

                    SectionCard(title: "Sent To Address", rows: [sentTo])
                    divider

                    SectionCard(title: "UTXO Change", rows: change)
                    divider

                    HStack {
                        Text("Fee")
                            .font(.caption)
                            .foregroundStyle(.secondary.opacity(0.75))

                        Spacer()
                        Text(fee)
                            .font(.footnote)
                            .fontWeight(.regular)
                    }
                    .padding(.horizontal, 12)
                }
            }
        }
        .padding(.horizontal)
        .background(Color(UIColor.secondarySystemBackground))
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }
}

private struct TxRow: View {
    let model: TxRowModel

    var body: some View {
        HStack(alignment: .top) {
            Text(model.address)
                .font(.caption2.monospaced())
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .multilineTextAlignment(.leading)
            Spacer(minLength: 12)
            Text(model.amountBTC)
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
                    .font(.caption.weight(.semibold))
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
    SendFlowAdvancedDetailsView(
        inputs: Array(
            repeating: TxRowModel(
                address: "bc1q uuye 0qg5 vyd3 e63s 0vus eqod 7h3j 44y1 8h4s 183d x37a",
                amountBTC: "0.0135 BTC"), count: 3),
        sentTo: TxRowModel(
            address: "bc1q uuye 0qg5 vyd3 e63s 0vus eqod 7h3j 44y1 8h4s 183d x37a",
            amountBTC: "0.0135 BTC"),
        change: Array(
            repeating: TxRowModel(
                address: "bc1q uuye 0qg5 vyd3 e63s 0vus eqod 7h3j 44y1 8h4s 183d x37a",
                amountBTC: "0.0135 BTC"), count: 3),
        fee: "0.0135 BTC"
    )
}
