import SwiftUI

// MARK: - View

struct UtxoListScreen: View {
    let manager: CoinControlManager

    @FocusState private var isFocused: Bool

    @ViewBuilder
    func UTXOList() -> some View {
        VStack(spacing: 0) {
            List(selection: manager.selectedBinding) {
                ForEach(manager.utxos) { utxo in
                    UTXORow(manager: manager, utxo: utxo)
                        .listRowBackground(Color.secondarySystemGroupedBackground)
                }
            }
            .scrollContentBackground(.hidden)
            .padding(.top, -35) // undo list default padding top
            .padding(.horizontal, -16) // undo default padding horizontal
            .environment(\.editMode, .constant(.active))
            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        }
        .background(manager.utxos.count < 6 ? Color.clear : Color.secondarySystemGroupedBackground)
        .clipShape(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
        )
        .padding(.horizontal)
    }

    var continueText: String {
        if manager.selected.count <= 1 { return "Continue" }
        return "Continue (\(manager.selected.count))"
    }

    // ─── Body ────────────────────────────────────────────────
    var body: some View {
        VStack(spacing: 24) {
            VStack(spacing: 16) {
                // ─ Search bar ─
                HStack {
                    Image(systemName: "magnifyingglass")
                    TextField("Search UTXOs", text: manager.searchBinding)
                        .focused($isFocused)
                        .autocorrectionDisabled()

                    if !manager.searchBinding.wrappedValue.isEmpty {
                        Button(action: { manager.dispatch(.clearSearch) }) {
                            Image(systemName: "xmark.circle.fill")
                                .foregroundColor(.gray)
                        }
                        .buttonStyle(PlainButtonStyle())
                        .transition(.scale)
                    }
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
                    Spacer()
                    sortButton(for: .change)
                }
                .padding(.horizontal)
            }

            VStack(spacing: 8) {
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
                UTXOList()
            }

            Spacer()

            // ─ Footer notes ─
            VStack(spacing: 16) {
                HStack {
                    Text(
                        "Select UTXOs to manage or send. Unspent outputs will remain in your wallet for future use."
                    )
                    .font(.caption)
                    .fontWeight(.regular)

                    Spacer()
                }

                HStack(spacing: 4) {
                    Image(systemName: "circlebadge.2")
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

            // ─ Action buttons ─
            Button(continueText) { /* … */ }
                .buttonStyle(
                    manager.selected.isEmpty
                        ? DarkButtonStyle(
                            backgroundColor: .systemGray4, foregroundColor: .secondary
                        )
                        : DarkButtonStyle()
                )
                .controlSize(.large)
                .frame(maxWidth: .infinity)
                .padding(.horizontal)
                .padding(.bottom, 4)
                .disabled(manager.selected.isEmpty)
                .padding(.horizontal)
        }
        .navigationTitle("Manage UTXOs")
        .background(
            Image(.utxoManagementPattern)
                .ignoresSafeArea()
                .opacity(0.85)
        )
        .background(
            Color(.systemGroupedBackground)
                .ignoresSafeArea()
        )
        .environment(manager)
    }

    // MARK: - Helpers

    private func sortButton(for key: CoinControlListSortKey) -> some View {
        Button {
            manager.dispatch(.changeSort(key))
        } label: {
            HStack {
                Text(key.title)

                if let arrow = manager.buttonArrow(key) {
                    Image(systemName: arrow)
                        .contentTransition(.symbolEffect)
                }
            }
            .font(.footnote)
            .fontWeight(.medium)
            .padding(.vertical, 8)
            .padding(.horizontal, 12)
            .background(manager.buttonColor(key))
            .foregroundColor(manager.buttonTextColor(key))
            .cornerRadius(100)
            .contentTransition(.interpolate)
            .lineLimit(1)
            .minimumScaleFactor(0.01)
        }
        .buttonStyle(.plain)
        .opacity(1)
    }
}

// MARK: - Row

private struct UTXORow: View {
    var manager: CoinControlManager
    let utxo: Utxo

    var body: some View {
        HStack(spacing: 0) {
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
                .frame(maxWidth: screenWidth * 0.35)
            }

            Spacer(minLength: 8)

            VStack(alignment: .trailing, spacing: 4) {
                Text(manager.displayAmount(utxo.amount))
                    .font(.footnote)
                    .fontWeight(.regular)

                Text(utxo.date)
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .padding(.vertical, 4)
    }
}

// MARK: - Preview

#Preview {
    AsyncPreview {
        UtxoListScreen(
            manager: CoinControlManager(RustCoinControlManager.previewNew())
        )
    }
}
