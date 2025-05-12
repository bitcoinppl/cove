import SwiftUI

// MARK: - Model

struct UTXO: Identifiable, Hashable {
    enum Kind { case change, regular }

    let id = UUID()
    let name: String
    let address: String
    let amountBTC: Double
    let date: Date
    let kind: Kind
}

// MARK: - View

struct ManageUTXOsView: View {
    // ─── Sample data ─────────────────────────────────────────
    @State private var utxos: [UTXO] = [
        .init(name: "Open SATs Payment",
              address: "bc1q uyye…e63s 0vus",
              amountBTC: 0.0135,
              date: .now,
              kind: .regular),
        .init(name: "Received",
              address: "bc1q uyye…e63s 0vus",
              amountBTC: 0.0135,
              date: .now,
              kind: .regular),
        .init(name: "Facebook Marketplace",
              address: "bc1q uyye…e63s 0vus",
              amountBTC: 0.0135,
              date: .now,
              kind: .regular),
        .init(name: "Change",
              address: "bc1q uyye…e63s 0vus",
              amountBTC: 0.0135,
              date: .now,
              kind: .change),
        .init(name: "Open SATs Payment",
              address: "bc1q uyye…e63s 0vus",
              amountBTC: 0.0135,
              date: .now,
              kind: .regular),
    ]

    // ─── UI state ────────────────────────────────────────────
    @State private var selected = Set<UTXO.ID>()
    @State private var search = ""
    @State private var sortKey = SortKey.date

    // ─── Sort helper ─────────────────────────────────────────
    enum SortKey: String, CaseIterable, Identifiable {
        case date, name, amount
        var id: Self { self }
        var title: String {
            switch self {
            case .date: "Date"
            case .name: "Name"
            case .amount: "Amount"
            }
        }

        func compare(_ a: UTXO, _ b: UTXO) -> Bool {
            switch self {
            case .date: a.date > b.date
            case .name: a.name < b.name
            case .amount: a.amountBTC > b.amountBTC
            }
        }
    }

    // ─── Body ────────────────────────────────────────────────
    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                // ─ Search bar ─
                HStack {
                    Image(systemName: "magnifyingglass")
                    TextField("Search Assets", text: $search)
                        .autocorrectionDisabled()
                }
                .padding(8)
                .background(Color(.secondarySystemFill))
                .cornerRadius(8)
                .padding(.horizontal)
                .padding(.top)

                // ─ Sort buttons ─
                HStack {
                    Spacer()
                    sortButton(for: .date)
                    Spacer()
                    sortButton(for: .name)
                    Spacer()
                    sortButton(for: .amount)
                    Spacer()
                }
                .padding(.top, 4)

                // ─ Section header ─
                HStack {
                    Text("LIST OF ASSETS")
                        .font(.footnote)
                        .fontWeight(.regular)
                        .foregroundColor(.primary)
                    Spacer()
                }
                .padding(.horizontal)
                .padding(.top, 12)

                // ─ UTXO list ─
                List(filteredUTXOs, selection: $selected) { utxo in
                    UTXORow(utxo: utxo)
                        .listRowBackground(Color.white)
                }
                .scrollContentBackground(.hidden)
                .environment(\.editMode, .constant(.active))
                .background(Color.white)
                .clipShape(RoundedRectangle(cornerRadius: 12))
                .padding(.horizontal)

                // ─ Footer notes ─
                VStack(spacing: 8) {
                    Text("Select UTXOs to manage or send. Unspent outputs will remain in your wallet for future use.")
                        .font(.footnote)
                        .fontWeight(.regular)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal)

                    HStack(spacing: 4) {
                        Image(systemName: "bitcoinsign.circle.fill")
                            .font(.footnote)
                        Text("Denotes UTXO change")
                            .font(.footnote)
                            .fontWeight(.regular)
                    }
                }
                .padding(.bottom, 12)

                // ─ Action buttons ─
                Button("Continue") { /* … */ }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .frame(maxWidth: .infinity)
                    .padding(.horizontal)
                    .padding(.bottom, 4)
                    .disabled(selected.isEmpty)

                Button("Customize Selection") { /* … */ }
                    .font(.callout)
                    .padding(.bottom, 12)
            }
            .navigationTitle("Manage UTXOs")
            .background(
                Image(.utxoManagementPattern)
                    .ignoresSafeArea()
            )
            .background(
                Color(.systemGroupedBackground)
                    .ignoresSafeArea()
            )
        }
    }

    // MARK: - Helpers

    private func sortButton(for key: SortKey) -> some View {
        Button {
            sortKey = key
        } label: {
            Text(key.title)
                .font(.footnote)
                .fontWeight(.regular)
                .frame(minWidth: 60)
                .padding(.vertical, 6)
                .background(sortKey == key
                    ? Color.accentColor
                    : Color(.secondarySystemFill)
                )
                .foregroundColor(sortKey == key
                    ? .white
                    : .primary
                )
                .cornerRadius(8)
        }
        .buttonStyle(.plain)
    }

    private var filteredUTXOs: [UTXO] {
        utxos
            .filter { search.isEmpty || $0.name.localizedCaseInsensitiveContains(search) }
            .sorted(by: sortKey.compare)
    }
}

// MARK: - Row

private struct UTXORow: View {
    let utxo: UTXO

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                // Name
                Text(utxo.name)
                    .font(.caption)
                    .fontWeight(.semibold)

                // Address (semi-bold caption)
                Text(utxo.address)
                    .font(.caption)
                    .fontWeight(.semibold)
                    .foregroundColor(.primary)
            }

            Spacer(minLength: 8)

            VStack(alignment: .trailing, spacing: 2) {
                // Amount (regular footnote)
                Text(utxo.amountBTC, format: .currency(code: "BTC"))
                    .font(.footnote)
                    .fontWeight(.regular)

                // Date (caption, secondary)
                Text(utxo.date, format: .dateTime.year().month().day())
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .padding(.vertical, 4)
    }
}

// MARK: - Preview

#Preview {
    ManageUTXOsView()
}
