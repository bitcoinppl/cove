//
//  UtxoListScreen.swift
//  Cove
//
//  Created by Praveen Perera on 5/19/25.
//

import SwiftUI

// MARK: - View

struct UtxoListScreen: View {
    @Environment(WalletManager.self) private var walletManager
    @Environment(\.navigate) private var navigate
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.openURL) private var openURL

    let manager: CoinControlManager

    @FocusState private var isFocused: Bool
    @State private var showLockedSelectionAlert = false
    @State private var utxoLockUpdateError: String? = nil

    private var continueText: String {
        if manager.selected.count <= 1 { return "Continue" }
        return "Continue (\(manager.selected.count))"
    }

    private var canContinue: Bool {
        !manager.selected.isEmpty && manager.totalSelectedSats > 0
    }

    var body: some View {
        UtxoListNavigationView(
            manager: manager,
            isFocused: $isFocused,
            continueTitle: continueText,
            canContinue: canContinue,
            colorScheme: colorScheme,
            onContinue: continueToSend,
            onShowTransaction: goToTransactionDetails,
            showLockedSelectionAlert: $showLockedSelectionAlert,
            utxoLockUpdateError: $utxoLockUpdateError
        )
    }

    private func goToTransactionDetails(_ utxo: Utxo) {
        let txId = utxo.id.txid()
        let walletId = walletManager.walletMetadata.id

        navigate(Route.transactionDetails(id: walletId, txId: txId))
    }

    private func continueToSend() {
        manager.continuePressed()
        navigate(
            RouteFactory()
                .coinControlSend(
                    id: manager.id,
                    utxos: manager.selectedUtxos()
                )
        )
    }
}

private struct UtxoListNavigationView: View {
    let manager: CoinControlManager
    let isFocused: FocusState<Bool>.Binding
    let continueTitle: String
    let canContinue: Bool
    let colorScheme: ColorScheme
    let onContinue: () -> Void
    let onShowTransaction: (Utxo) -> Void

    @Binding var showLockedSelectionAlert: Bool
    @Binding var utxoLockUpdateError: String?

    var body: some View {
        UtxoListAlertsView(
            manager: manager,
            isFocused: isFocused,
            continueTitle: continueTitle,
            canContinue: canContinue,
            onContinue: onContinue,
            onShowTransaction: onShowTransaction,
            showLockedSelectionAlert: $showLockedSelectionAlert,
            utxoLockUpdateError: $utxoLockUpdateError
        )
        .navigationTitle("Manage UTXOs")
        .navigationBarTitleDisplayMode(isFocused.wrappedValue ? .inline : .automatic)
        .toolbar {
            ToolbarItemGroup(placement: .topBarTrailing) {
                UtxoListToolbarMenu(
                    selectionIsEmpty: manager.selected.isEmpty,
                    onToggleUnit: { manager.dispatch(.toggleUnit) },
                    onToggleSelectAll: { manager.dispatch(.toggleSelectAll) }
                )
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
    }
}

private struct UtxoListAlertsView: View {
    let manager: CoinControlManager
    let isFocused: FocusState<Bool>.Binding
    let continueTitle: String
    let canContinue: Bool
    let onContinue: () -> Void
    let onShowTransaction: (Utxo) -> Void

    @Binding var showLockedSelectionAlert: Bool
    @Binding var utxoLockUpdateError: String?

    private var updateErrorIsPresented: Binding<Bool> {
        Binding(
            get: { utxoLockUpdateError != nil },
            set: { isPresented in
                if !isPresented {
                    utxoLockUpdateError = nil
                }
            }
        )
    }

    var body: some View {
        UtxoListContent(
            manager: manager,
            isFocused: isFocused,
            onShowTransaction: onShowTransaction,
            showLockedSelectionAlert: $showLockedSelectionAlert,
            utxoLockUpdateError: $utxoLockUpdateError
        )
        .overlay(alignment: .bottom) {
            if !isFocused.wrappedValue {
                UtxoContinueButton(
                    title: continueTitle,
                    isEnabled: canContinue,
                    action: onContinue
                )
                .padding(.horizontal)
                .padding(.bottom, 32)
            }
        }
        .alert("UTXO Locked", isPresented: $showLockedSelectionAlert) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("Unlock this UTXO before selecting it.")
        }
        .alert("Unable to Update UTXO Lock", isPresented: updateErrorIsPresented) {
            Button("OK", role: .cancel) {
                utxoLockUpdateError = nil
            }
        } message: {
            Text(utxoLockUpdateError ?? "")
        }
        .environment(manager)
        .task {
            await manager.reloadLabels()
        }
    }
}

private struct UtxoListContent: View {
    let manager: CoinControlManager
    let isFocused: FocusState<Bool>.Binding
    let onShowTransaction: (Utxo) -> Void

    @Binding var showLockedSelectionAlert: Bool
    @Binding var utxoLockUpdateError: String?

    var body: some View {
        VStack(spacing: 24) {
            UtxoSearchAndSortControls(manager: manager, isFocused: isFocused)

            UtxoSelectionSection(
                manager: manager,
                onShowTransaction: onShowTransaction,
                showLockedSelectionAlert: $showLockedSelectionAlert,
                utxoLockUpdateError: $utxoLockUpdateError
            )

            Spacer()

            if !isFocused.wrappedValue {
                UtxoListFooter()
            }
        }
        .frame(maxHeight: .infinity)
        .padding(.bottom, isFocused.wrappedValue ? 0 : 112)
    }
}

private struct UtxoSearchAndSortControls: View {
    let manager: CoinControlManager
    let isFocused: FocusState<Bool>.Binding

    var body: some View {
        VStack(spacing: 16) {
            UtxoSearchBar(
                search: manager.searchBinding,
                isFocused: isFocused,
                onClear: { manager.dispatch(.clearSearch) }
            )

            if !isFocused.wrappedValue {
                UtxoSortControls(manager: manager)
            }
        }
    }
}

private struct UtxoSearchBar: View {
    let search: Binding<String>
    let isFocused: FocusState<Bool>.Binding
    let onClear: () -> Void

    var body: some View {
        HStack {
            Image(systemName: "magnifyingglass")
            TextField("Search UTXOs", text: search)
                .focused(isFocused)
                .autocorrectionDisabled()
                .autocapitalization(.none)

            if !search.wrappedValue.isEmpty {
                Button(action: onClear) {
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
    }
}

private struct UtxoSortControls: View {
    let manager: CoinControlManager

    var body: some View {
        HStack {
            UtxoSortButton(manager: manager, key: .date)
            Spacer()
            UtxoSortButton(manager: manager, key: .name)
            Spacer()
            UtxoSortButton(manager: manager, key: .amount)
            Spacer()
            UtxoSortButton(manager: manager, key: .change)
        }
        .padding(.horizontal)
    }
}

private struct UtxoSortButton: View {
    let manager: CoinControlManager
    let key: CoinControlListSortKey

    var body: some View {
        let _ = manager.sort
        let isSelected = manager.isSortSelected(key)

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
            .background(isSelected ? Color.statusInfo : Color.systemGray5)
            .foregroundColor(isSelected ? .white : .secondary.opacity(0.60))
            .cornerRadius(100)
            .contentTransition(.interpolate)
            .lineLimit(1)
            .minimumScaleFactor(0.01)
        }
        .buttonStyle(.plain)
        .opacity(1)
    }
}

private struct UtxoSelectionSection: View {
    let manager: CoinControlManager
    let onShowTransaction: (Utxo) -> Void

    @Binding var showLockedSelectionAlert: Bool
    @Binding var utxoLockUpdateError: String?

    var body: some View {
        VStack(spacing: 8) {
            UtxoSelectionHeader(
                selectionIsEmpty: manager.selected.isEmpty,
                onToggleSelectAll: { manager.dispatch(.toggleSelectAll) }
            )

            UtxoListView(
                manager: manager,
                onShowTransaction: onShowTransaction,
                showLockedSelectionAlert: $showLockedSelectionAlert,
                utxoLockUpdateError: $utxoLockUpdateError
            )

            if manager.lockStateLoadFailed {
                Text("Unable to read UTXO lock state. UTXOs are shown locked for safety.")
                    .font(.caption)
                    .fontWeight(.medium)
                    .foregroundStyle(.red)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal)
            }

            Text(manager.totalSelectedAmount)
                .font(.caption2.weight(.semibold))
                .foregroundStyle(.secondary)
                .opacity(manager.selected.isEmpty ? 0 : 0.8)
                .contentTransition(.numericText())
                .animation(.easeInOut(duration: 0.1), value: manager.totalSelectedAmount)
        }
    }
}

private struct UtxoSelectionHeader: View {
    let selectionIsEmpty: Bool
    let onToggleSelectAll: () -> Void

    var body: some View {
        HStack {
            Text("LIST OF UTXOS")
                .font(.caption)
                .fontWeight(.regular)
                .foregroundColor(.primary.opacity(0.6))
            Spacer()

            Text(selectionIsEmpty ? "Select All" : "Deselect All")
                .font(.caption)
                .fontWeight(.medium)
                .foregroundStyle(.link)
                .contentShape(
                    Rectangle().inset(
                        by: EdgeInsets(
                            top: -15,
                            leading: -35,
                            bottom: -10,
                            trailing: -35
                        )
                    )
                )
                .onTapGesture(perform: onToggleSelectAll)
        }
        .padding(.horizontal)
        .padding(.horizontal)
        .zIndex(1)
    }
}

private struct UtxoListView: View {
    let manager: CoinControlManager
    let onShowTransaction: (Utxo) -> Void

    @Binding var showLockedSelectionAlert: Bool
    @Binding var utxoLockUpdateError: String?

    var body: some View {
        VStack(spacing: 0) {
            List(selection: manager.selectedBinding) {
                ForEach(manager.utxos) { utxo in
                    UtxoListItem(
                        manager: manager,
                        utxo: utxo,
                        onLockedSelectionAttempt: { showLockedSelectionAlert = true },
                        onShowTransaction: { onShowTransaction(utxo) },
                        onLockUpdateError: { utxoLockUpdateError = $0 }
                    )
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
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        .padding(.horizontal)
    }
}

private struct UtxoListItem: View {
    let manager: CoinControlManager
    let utxo: Utxo
    let onLockedSelectionAttempt: () -> Void
    let onShowTransaction: () -> Void
    let onLockUpdateError: (String) -> Void

    var body: some View {
        UtxoRow(
            manager: manager,
            utxo: utxo,
            onLockedSelectionAttempt: onLockedSelectionAttempt
        )
        .selectionDisabled(!utxo.spendable)
        .listRowBackground(Color.secondarySystemGroupedBackground)
        .contextMenu {
            UtxoContextMenu(
                utxo: utxo,
                onShowTransaction: onShowTransaction,
                onToggleLock: toggleLock
            )
        } preview: {
            UtxoRowPreview(displayAmount: manager.displayAmount, utxo: utxo)
        }
    }

    private func toggleLock() {
        Task {
            do {
                try await manager.setSpendability(!utxo.spendable, for: utxo.outpoint)
            } catch {
                Log.error("Unable to update UTXO spendability: \(error)")
                await MainActor.run {
                    onLockUpdateError(error.localizedDescription)
                }
            }
        }
    }
}

private struct UtxoContextMenu: View {
    let utxo: Utxo
    let onShowTransaction: () -> Void
    let onToggleLock: () -> Void

    var body: some View {
        Button("Copy Address", action: copyAddress)
        Button("Copy Transaction ID", action: copyTransactionId)
        Button("View Transaction Details", action: onShowTransaction)
        Button(utxo.spendable ? "Lock UTXO" : "Unlock UTXO", action: onToggleLock)
    }

    private func copyAddress() {
        UIPasteboard.general.string = utxo.address.unformatted()
    }

    private func copyTransactionId() {
        UIPasteboard.general.string = utxo.outpoint.txidStr()
    }
}

private struct UtxoListFooter: View {
    var body: some View {
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
    }
}

private struct UtxoContinueButton: View {
    let title: String
    let isEnabled: Bool
    let action: () -> Void

    var body: some View {
        Button(title, action: action)
            .buttonStyle(
                isEnabled
                    ? DarkButtonStyle()
                    : DarkButtonStyle(
                        backgroundColor: .systemGray4,
                        foregroundColor: .secondary
                    )
            )
            .controlSize(.large)
            .frame(maxWidth: .infinity)
            .disabled(!isEnabled)
            .contentTransition(.interpolate)
    }
}

private struct UtxoListToolbarMenu: View {
    let selectionIsEmpty: Bool
    let onToggleUnit: () -> Void
    let onToggleSelectAll: () -> Void

    var body: some View {
        Menu("More", systemImage: "ellipsis") {
            Button("Toggle Unit", action: onToggleUnit)
            Button(
                selectionIsEmpty ? "Select All" : "Deselect All",
                action: onToggleSelectAll
            )
        }
        .foregroundColor(.primary)
        .tint(.primary)
    }
}

// MARK: - Row

private struct UtxoRow: View {
    var manager: CoinControlManager
    let utxo: Utxo
    let onLockedSelectionAttempt: () -> Void

    var body: some View {
        HStack(spacing: 20) {
            VStack(alignment: .leading, spacing: 4) {
                // Name
                HStack(spacing: 4) {
                    Text(utxo.name())
                        .font(.footnote)
                        .truncationMode(.middle)
                        .lineLimit(1)

                    if utxo.type == .change {
                        Image(systemName: "circlebadge.2")
                            .font(.caption)
                            .foregroundColor(.statusWarning.opacity(0.8))
                    }

                    if !utxo.spendable {
                        Image(systemName: "lock.fill")
                            .font(.caption)
                            .foregroundColor(.secondary)
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

                Text(utxo.date())
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .padding(.vertical, 4)
        .opacity(utxo.spendable ? 1 : 0.58)
        .contentShape(Rectangle())
        .simultaneousGesture(
            TapGesture().onEnded {
                guard !utxo.spendable else { return }
                onLockedSelectionAttempt()
            }
        )
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
                RustCoinControlManager.previewNew(outputCount: 0, changeCount: 0)
            )
        )
        .environment(WalletManager(preview: "preview_only"))
    }
}
