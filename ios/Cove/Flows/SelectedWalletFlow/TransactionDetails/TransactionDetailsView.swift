//
//  TransactionDetailsView.swift
//  Cove
//
//  Created by Praveen Perera on 9/17/24.
//

import SwiftUI

private let lockStateUpdateRevealDelay: Duration = .milliseconds(200)
private let lockStateUpdateMinimumVisibleDuration: Duration = .milliseconds(350)

struct TransactionDetailsView: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.openURL) private var openURL
    @Environment(\.sizeCategory) var sizeCategory

    @State private var scrollPosition = ScrollPosition()

    @State private var initialOffset: Double? = nil
    @State private var currentOffset: Double = 0
    @State private var isUpdatingLockState = false
    @State private var showLockStateUpdatingIndicator = false
    @State private var lockStateUpdatingIndicatorShownAt: ContinuousClock.Instant? = nil
    @State private var lockStateError: String? = nil
    @State private var lockStateLoadError: String? = nil

    // public
    let id: WalletId
    let txId: TxId
    private let initialPresentation: TransactionDetailsPresentation
    let refreshOnAppear: Bool
    var manager: WalletManager

    /// read from cache (observable), fallback to the initial presentation
    var transactionDetailsPresentation: TransactionDetailsPresentation {
        manager.transactionDetailsPresentations[txId] ?? initialPresentation
    }

    var transactionDetails: TransactionDetails {
        transactionDetailsPresentation.details()
    }

    var lockState: TransactionLockState? {
        manager.transactionLockStates[txId]
    }

    var numberOfConfirmations: Int? {
        transactionDetailsPresentation.confirmations().map(Int.init)
    }

    init(
        id: WalletId,
        txId: TxId,
        transactionDetailsPresentation: TransactionDetailsPresentation,
        refreshOnAppear: Bool = true,
        manager: WalletManager
    ) {
        self.id = id
        self.txId = txId
        self.initialPresentation = transactionDetailsPresentation
        self.refreshOnAppear = refreshOnAppear
        self.manager = manager
    }

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    var detailsExpanded: Bool {
        metadata.detailsExpanded
    }

    func ContentScrollView(content: @escaping () -> some View) -> some View {
        GeometryReader { geo in
            ScrollView(.vertical) {
                content()
                    .frame(minHeight: geo.size.height)
            }
            .scrollIndicators(.never)
            .frame(alignment: .top)
            .scrollPosition($scrollPosition)
            .onScrollGeometryChange(for: Double.self) { geo in
                geo.contentOffset.y
            } action: { oldValue, newValue in
                if oldValue == newValue { return }
                if oldValue == 0 { return }
                let initialOffset = initialOffset ?? oldValue
                self.initialOffset = initialOffset
                currentOffset = initialOffset - newValue
            }
        }
    }

    private func retryTransactionLockState() {
        Task { await refreshTransactionLockState() }
    }

    private func beginToggleTransactionLockState() {
        guard !isUpdatingLockState else { return }

        isUpdatingLockState = true
        showLockStateUpdatingIndicator = false
        lockStateUpdatingIndicatorShownAt = nil
        Task {
            await updateTransactionLockState {
                try await manager.toggleTransactionLockState(for: txId)
            }
        }
    }

    private func beginUnlockTransactionOutputs() {
        guard !isUpdatingLockState else { return }

        isUpdatingLockState = true
        showLockStateUpdatingIndicator = false
        lockStateUpdatingIndicatorShownAt = nil
        Task {
            await updateTransactionLockState {
                try await manager.unlockTransactionOutputs(for: txId)
            }
        }
    }

    var body: some View {
        ContentScrollView {
            VStack(spacing: 24) {
                if sizeCategory < .extraExtraExtraLarge || isMiniDevice { Spacer() }

                Group {
                    if transactionDetails.isReceived() {
                        TransactionReceivedDetailsSection(
                            transactionDetails: transactionDetails,
                            manager: manager,
                            metadata: metadata,
                            numberOfConfirmations: numberOfConfirmations,
                            lockState: lockState,
                            isUpdatingLockState: isUpdatingLockState,
                            showLockStateUpdatingIndicator: showLockStateUpdatingIndicator,
                            lockStateLoadError: lockStateLoadError,
                            retryLockState: retryTransactionLockState,
                            requestUnlockLockedUtxos: beginUnlockTransactionOutputs,
                            toggleLockState: beginToggleTransactionLockState
                        )
                    } else {
                        TransactionSentDetailsSection(
                            transactionDetails: transactionDetails,
                            manager: manager,
                            metadata: metadata,
                            numberOfConfirmations: numberOfConfirmations,
                            lockState: lockState,
                            isUpdatingLockState: isUpdatingLockState,
                            showLockStateUpdatingIndicator: showLockStateUpdatingIndicator,
                            lockStateLoadError: lockStateLoadError,
                            retryLockState: retryTransactionLockState,
                            requestUnlockLockedUtxos: beginUnlockTransactionOutputs,
                            toggleLockState: beginToggleTransactionLockState
                        )
                    }
                }

                Spacer()
                if sizeCategory < .extraExtraLarge || isMiniDevice { Spacer() }
                if !isMiniDevice, sizeCategory < .extraLarge { Spacer() }

                Button(action: {
                    if let url = URL(string: transactionDetails.transactionUrl()) {
                        openURL(url)
                    }
                }) {
                    Text("View in Explorer")
                        .frame(maxWidth: .infinity)
                        .padding()
                        .background(Color.midnightBtn)
                        .foregroundColor(.white)
                        .cornerRadius(10)
                        .padding(.horizontal, detailsExpandedPadding)
                }

                Button(action: {
                    if detailsExpanded {
                        withAnimation { scrollPosition.scrollTo(edge: .top) }
                        manager.dispatch(action: .toggleDetailsExpanded)
                    } else {
                        manager.dispatch(action: .toggleDetailsExpanded)
                    }
                }) {
                    Text(detailsExpanded ? "Hide Details" : "Show Details")
                        .font(.footnote)
                        .fontWeight(.bold)
                        .foregroundStyle(Color.secondary.opacity(0.8))
                }
                .padding(.top, 10)
                .offset(y: -20)
            }
        }
        .refreshable {
            await refreshTransactionDetails()
            await refreshTransactionLockState()
        }
        .task(id: txId) {
            // fetch fresh details on load
            if refreshOnAppear {
                await refreshTransactionDetails()
            }

            await refreshTransactionLockState()
        }
        .background(
            GeometryReader { geometry in
                Image(.transactionDetailsPattern)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .frame(width: geometry.size.width, alignment: .center)
                    .ignoresSafeArea(edges: .top)
                    .opacity(colorScheme == .light ? 0.40 : 1)
                    .offset(y: currentOffset > 0 ? 0 : currentOffset)
                    .opacity(max(0, 1 + (currentOffset / 275)))
            }
        )
        .onAppear {
            UIRefreshControl.appearance().tintColor = colorScheme == .light ? UIColor.label : UIColor.secondaryLabel
        }
        .onChange(of: colorScheme) { _, newScheme in
            UIRefreshControl.appearance().tintColor = newScheme == .light ? UIColor.label : UIColor.secondaryLabel
        }
        .onDisappear {
            UIRefreshControl.appearance().tintColor = UIColor.secondaryLabel
        }
        .alert("Unable to Update Lock", isPresented: Binding(
            get: { lockStateError != nil },
            set: { if !$0 { lockStateError = nil } }
        )) {
            Button("OK", role: .cancel) {
                lockStateError = nil
            }
        } message: {
            Text(lockStateError ?? "")
        }
    }

    func refreshTransactionDetails() async {
        do {
            _ = try await manager.refreshTransactionDetails(for: txId)
        } catch {
            Log.error("Error refreshing transaction details: \(error)")
        }
    }

    func refreshTransactionLockState() async {
        do {
            _ = try await manager.transactionLockState(for: txId)
            await MainActor.run {
                lockStateLoadError = nil
            }
        } catch {
            Log.error("Error refreshing transaction lock state: \(error)")
            await MainActor.run {
                lockStateLoadError = error.localizedDescription
            }
        }
    }

    func updateTransactionLockState(
        operation: @escaping () async throws -> TransactionLockState
    ) async {
        let indicatorTask = Task {
            do {
                try await Task.sleep(for: lockStateUpdateRevealDelay)
                try Task.checkCancellation()
                await MainActor.run {
                    lockStateUpdatingIndicatorShownAt = ContinuousClock.now
                    showLockStateUpdatingIndicator = true
                }
            } catch is CancellationError {
                return
            } catch {
                Log.error("Error showing transaction lock update indicator: \(error)")
            }
        }
        var updateError: String? = nil

        do {
            _ = try await operation()
        } catch {
            updateError = error.localizedDescription
        }

        indicatorTask.cancel()

        if let visibleSince = await MainActor.run(body: { lockStateUpdatingIndicatorShownAt }) {
            let remaining = lockStateUpdateMinimumVisibleDuration - visibleSince.duration(to: ContinuousClock.now)
            if remaining > .zero {
                try? await Task.sleep(for: remaining)
            }
        }

        await MainActor.run {
            showLockStateUpdatingIndicator = false
            lockStateUpdatingIndicatorShownAt = nil
            isUpdatingLockState = false

            if let updateError {
                lockStateError = updateError
            }
        }
    }

    var backgroundImageOffset: CGFloat {
        guard detailsExpanded else { return 0 }
        guard currentOffset < 0 else { return 0 }
        return currentOffset
    }
}

#Preview("confirmed received") {
    AsyncPreview {
        let presentation = TransactionDetailsPresentation.previewConfirmedReceived()
        let details = presentation.details()

        TransactionDetailsView(
            id: WalletId(),
            txId: details.txId(),
            transactionDetailsPresentation: presentation,
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}

#Preview("confirmed sent") {
    AsyncPreview {
        let presentation = TransactionDetailsPresentation.previewConfirmedSent()
        let details = presentation.details()

        TransactionDetailsView(
            id: WalletId(),
            txId: details.txId(),
            transactionDetailsPresentation: presentation,
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}

#Preview("pending received") {
    AsyncPreview {
        let presentation = TransactionDetailsPresentation.previewPendingReceived()
        let details = presentation.details()

        TransactionDetailsView(
            id: WalletId(),
            txId: details.txId(),
            transactionDetailsPresentation: presentation,
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}

#Preview("pending sent") {
    AsyncPreview {
        let presentation = TransactionDetailsPresentation.previewPendingSent()
        let details = presentation.details()

        TransactionDetailsView(
            id: WalletId(),
            txId: details.txId(),
            transactionDetailsPresentation: presentation,
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}
