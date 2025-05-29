//
//  UtxoListScreen.swift
//  Cove
//
//  Created by Praveen Perera on 5/19/25.
//

import MijickPopupView
import SwiftUI

// MARK: - View

struct UtxoListScreen: View {
    @Environment(WalletManager.self) private var walletManager
    @Environment(\.navigate) private var navigate
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.openURL) private var openURL

    let manager: CoinControlManager

    @FocusState private var isFocused: Bool

    func goToTransactionDetails(_ utxo: Utxo) {
        let txId = utxo.id.txid()
        let walletId = walletManager.walletMetadata.id

        if let details = walletManager.transactionDetails[txId] {
            return navigate(Route.transactionDetails(id: walletId, details: details))
        }

        MiddlePopup(state: .loading).showAndStack()
        Task {
            do {
                let details = try await walletManager.transactionDetails(for: txId)
                await MainActor.run {
                    PopupManager.dismiss()
                    navigate(Route.transactionDetails(id: walletId, details: details))
                }
            } catch {
                Log.error(
                    "Unable to get transaction details: \(error.localizedDescription), for txn: \(txId)"
                )
            }
        }
    }

    @ViewBuilder
    func UtxoList() -> some View {
        VStack(spacing: 0) {
            List(selection: manager.selectedBinding) {
                ForEach(manager.utxos) { utxo in
                    UtxoRow(manager: manager, utxo: utxo)
                        .listRowBackground(Color.secondarySystemGroupedBackground)
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

                            Button(action: { goToTransactionDetails(utxo) }) {
                                Text("View Transaction Details")
                            }
                        } preview: {
                            UtxoRowPreview(displayAmount: manager.displayAmount, utxo: utxo)
                        }
                }
            }
            .scrollContentBackground(.hidden)
            .padding(.top, -35) // undo list default padding top
            .padding(.horizontal, -16) // undo default padding horizontal
            .environment(\.editMode, .constant(.active))
            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay {
                if manager.utxos.isEmpty {
                    ContentUnavailableView.search
                        .background(Color.secondarySystemGroupedBackground)
                }
            }
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
                        .autocapitalization(.none)

                    if !manager.search.isEmpty {
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
                if !isFocused {
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
            }

            VStack(spacing: 8) {
                // ─ Section header ─
                HStack {
                    Text("LIST OF UTXOS")
                        .font(.caption)
                        .fontWeight(.regular)
                        .foregroundColor(.primary.opacity(0.6))
                    Spacer()

                    Group {
                        if manager.selected.isEmpty {
                            Text("Select All")
                        } else {
                            Text("Deselect All")
                        }
                    }
                    .font(.caption)
                    .fontWeight(.medium)
                    .foregroundStyle(.blue)
                    .contentShape(
                        Rectangle().inset(
                            by:
                            EdgeInsets(
                                top: -15,
                                leading: -35,
                                bottom: -10,
                                trailing: -35
                            ))
                    )
                    .onTapGesture { manager.dispatch(.toggleSelectAll) }
                }
                .padding(.horizontal)
                .padding(.horizontal)
                .zIndex(1)

                // ─ UTXO list ─
                VStack(spacing: 8) {
                    UtxoList()
                    Text(manager.totalSelectedAmount)
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .opacity(manager.selected.isEmpty ? 0 : 0.8)
                        .contentTransition(.numericText())
                        .animation(.easeInOut(duration: 0.1), value: manager.totalSelectedAmount)
                }
            }

            Spacer()

            if !isFocused {
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
                Button(continueText) {
                    manager.continuePressed()
                    navigate(
                        RouteFactory()
                            .coinControlSend(
                                id: manager.rust.id(),
                                utxos: manager.rust.selectedUtxos(),
                            )
                    )
                }
                .buttonStyle(
                    manager.totalSelectedSats < minSendSats
                        ? DarkButtonStyle(
                            backgroundColor: .systemGray4, foregroundColor: .secondary
                        )
                        : DarkButtonStyle()
                )
                .controlSize(.large)
                .frame(maxWidth: .infinity)
                .padding(.horizontal)
                .padding(.bottom, 4)
                .disabled(manager.totalSelectedSats < minSendSats)
                .contentTransition(.interpolate)
            }
        }
        .navigationTitle("Manage UTXOs")
        .navigationBarTitleDisplayMode(isFocused ? .inline : .automatic)
        .toolbar {
            ToolbarItemGroup(placement: .topBarTrailing) {
                Menu("More", systemImage: "ellipsis") {
                    Button(action: { manager.dispatch(.toggleUnit) }) {
                        Text("Toggle Unit")
                    }

                    Button(action: { manager.dispatch(.toggleSelectAll) }) {
                        if manager.selected.isEmpty {
                            Text("Select All")
                        } else {
                            Text("Deselect All")
                        }
                    }
                }
                .foregroundColor(.primary)
                .tint(.primary)
            }
        }
        .background(
            Image(.utxoManagementPattern)
                .ignoresSafeArea()
                .opacity(colorScheme == .light ? 0.80 : 1)
        )
        .background(
            Color(.systemGroupedBackground)
                .ignoresSafeArea()
        )
        .environment(manager)
        .task { await manager.rust.reloadLabels() }
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

private struct UtxoRow: View {
    var manager: CoinControlManager
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
        .environment(WalletManager(preview: "preview_only"))
    }
}

#Preview("Empty") {
    AsyncPreview {
        UtxoListScreen(
            manager: CoinControlManager(
                RustCoinControlManager.previewNew(outputCount: 0, changeCount: 0))
        )
        .environment(WalletManager(preview: "preview_only"))
    }
}
