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
//        .init(name: "Facebook Marketplace",
//              address: "bc1q uyye…e63s 0vus",
//              amountBTC: 0.0135,
//              date: .now,
//              kind: .regular),
//        .init(name: "Change",
//              address: "bc1q uyye…e63s 0vus",
//              amountBTC: 0.0135,
//              date: .now,
//              kind: .change),
//        .init(name: "Open SATs Payment",
//              address: "bc1q uyye…e63s 0vus",
//              amountBTC: 0.0135,
//              date: .now,
//              kind: .regular),
//        .init(name: "Change",
//              address: "bc1q uyye…e63s 0vus",
//              amountBTC: 0.0135,
//              date: .now,
//              kind: .change),
//        .init(name: "Open SATs Payment",
//              address: "bc1q uyye…e63s 0vus",
//              amountBTC: 0.0135,
//              date: .now,
//              kind: .regular),
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
            VStack(spacing: 24) {
                VStack(spacing: 16) {
                    // ─ Search bar ─
                    HStack {
                        Image(systemName: "magnifyingglass")
                        TextField("Search UTXOs", text: $search)
                            .autocorrectionDisabled()
                    }
                    .padding(8)
                    .background(Color.systemGray5)
                    .cornerRadius(10)
                    .padding(.horizontal)
                    
                    // ─ Sort buttons ─
                    HStack {
                        sortButton(for: .date)
                        Spacer()
                        sortButton(for: .name)
                        Spacer()
                        sortButton(for: .amount)
                    }
                    .padding(.horizontal)
                }

                VStack(spacing: 8){
                    // ─ Section header ─
                    HStack {
                        Text("LIST OF UTXOS")
                            .font(.caption)
                            .fontWeight(.regular)
                            .foregroundColor(.primary.opacity(0.6))
                        Spacer()
                    }
                    .padding(.horizontal)
                    .padding(.horizontal)
                    
                    // ─ UTXO list ─
                    List(filteredUTXOs, selection: $selected) { utxo in
                        UTXORow(utxo: utxo).listRowBackground(Color.systemBackground)
                    }
                    .listStyle(.insetGrouped)
                    .environment(\.editMode, .constant(.active))
                    .padding(.horizontal)
                    .clipShape(RoundedRectangle(cornerRadius: 10))
                    .scrollContentBackground(.hidden)
                }
                
                // ─ Footer notes ─
                VStack(spacing: 16) {
                    HStack{
                        Text("Select UTXOs to manage or send. Unspent outputs will remain in your wallet for future use.")
                            .font(.caption)
                            .fontWeight(.regular)
                        
                        Spacer()
                    }

                    HStack(spacing: 4) {
                        Image(systemName: "bitcoinsign.circle.fill")
                            .font(.footnote)
                        
                        Text("Denotes UTXO change")
                            .font(.caption)
                            .fontWeight(.regular)
                        
                        Spacer()
                    }
                }
                .foregroundStyle(.secondary)
                .padding(.horizontal)
                .padding(.horizontal)
                
                Spacer()

                // ─ Action buttons ─
                Button("Continue") { /* … */ }
                    .buttonStyle(
                        selected.isEmpty ?
                         DarkButtonStyle(backgroundColor: .systemGray4, foregroundColor: .secondary) :
                                DarkButtonStyle()
                    )
                    .controlSize(.large)
                    .frame(maxWidth: .infinity)
                    .padding(.horizontal)
                    .padding(.bottom, 4)
                    .disabled(selected.isEmpty)
                    .padding(.horizontal)
                
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
        Button { sortKey = key }
        label: {
            Text(key.title)
                .font(.footnote)
                .fontWeight(.medium)
                .frame(minWidth: 60)
                .padding(.vertical, 8)
                .padding(.horizontal, 12)
                .background(sortKey == key ? .blue : .systemGray5)
                .foregroundColor(sortKey == key ? .white : .secondary.opacity(0.60))
                .cornerRadius(100)
        }
        .buttonStyle(.plain)
        .opacity(1)
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
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 4) {
                // Name
                Text(utxo.name)
                    .font(.footnote)

                // Address (semi-bold caption)
                Text(utxo.address)
                    .font(.caption2)
                    .fontWeight(.semibold)
                    .foregroundColor(.secondary)
                    .truncationMode(.middle)
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
