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
        TransactionDetailsScrollView(
            scrollPosition: $scrollPosition,
            initialOffset: $initialOffset,
            currentOffset: $currentOffset,
            content: TransactionDetailsContent(
                sizeCategory: sizeCategory,
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
                toggleLockState: beginToggleTransactionLockState,
                openExplorer: openTransactionExplorer,
                toggleDetails: toggleDetails
            )
        )
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
            TransactionDetailsBackground(
                colorScheme: colorScheme,
                currentOffset: currentOffset
            )
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
        .modifier(TransactionLockErrorModifier(error: $lockStateError))
    }

    private func openTransactionExplorer() {
        guard let url = URL(string: transactionDetails.transactionUrl()) else { return }

        openURL(url)
    }

    private func toggleDetails() {
        if detailsExpanded {
            withAnimation { scrollPosition.scrollTo(edge: .top) }
        }

        manager.dispatch(action: .toggleDetailsExpanded)
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

private struct TransactionDetailsScrollView<Content: View>: View {
    @Binding var scrollPosition: ScrollPosition
    @Binding var initialOffset: Double?
    @Binding var currentOffset: Double

    let content: Content

    var body: some View {
        GeometryReader { geometry in
            ScrollView(.vertical) {
                content
                    .frame(minHeight: geometry.size.height)
            }
            .scrollIndicators(.never)
            .frame(alignment: .top)
            .scrollPosition($scrollPosition)
            .onScrollGeometryChange(for: Double.self) { geometry in
                geometry.contentOffset.y
            } action: { oldValue, newValue in
                updateOffset(oldValue: oldValue, newValue: newValue)
            }
        }
    }

    private func updateOffset(oldValue: Double, newValue: Double) {
        guard oldValue != newValue, oldValue != 0 else { return }

        let initialOffset = initialOffset ?? oldValue
        self.initialOffset = initialOffset
        currentOffset = initialOffset - newValue
    }
}

private struct TransactionDetailsContent: View {
    let sizeCategory: ContentSizeCategory
    let transactionDetails: TransactionDetails
    let manager: WalletManager
    let metadata: WalletMetadata
    let numberOfConfirmations: Int?
    let lockState: TransactionLockState?
    let isUpdatingLockState: Bool
    let showLockStateUpdatingIndicator: Bool
    let lockStateLoadError: String?
    let retryLockState: () -> Void
    let requestUnlockLockedUtxos: () -> Void
    let toggleLockState: () -> Void
    let openExplorer: () -> Void
    let toggleDetails: () -> Void

    var body: some View {
        VStack(spacing: 24) {
            if sizeCategory < .extraExtraExtraLarge || isMiniDevice {
                Spacer()
            }

            TransactionDetailsDirectionContent(
                transactionDetails: transactionDetails,
                manager: manager,
                metadata: metadata,
                numberOfConfirmations: numberOfConfirmations,
                lockState: lockState,
                isUpdatingLockState: isUpdatingLockState,
                showLockStateUpdatingIndicator: showLockStateUpdatingIndicator,
                lockStateLoadError: lockStateLoadError,
                retryLockState: retryLockState,
                requestUnlockLockedUtxos: requestUnlockLockedUtxos,
                toggleLockState: toggleLockState
            )

            Spacer()
            if sizeCategory < .extraExtraLarge || isMiniDevice {
                Spacer()
            }
            if !isMiniDevice, sizeCategory < .extraLarge {
                Spacer()
            }

            TransactionDetailsFooterActions(
                detailsExpanded: metadata.detailsExpanded,
                openExplorer: openExplorer,
                toggleDetails: toggleDetails
            )
        }
    }
}

private struct TransactionDetailsDirectionContent: View {
    let transactionDetails: TransactionDetails
    let manager: WalletManager
    let metadata: WalletMetadata
    let numberOfConfirmations: Int?
    let lockState: TransactionLockState?
    let isUpdatingLockState: Bool
    let showLockStateUpdatingIndicator: Bool
    let lockStateLoadError: String?
    let retryLockState: () -> Void
    let requestUnlockLockedUtxos: () -> Void
    let toggleLockState: () -> Void

    var body: some View {
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
                retryLockState: retryLockState,
                requestUnlockLockedUtxos: requestUnlockLockedUtxos,
                toggleLockState: toggleLockState
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
                retryLockState: retryLockState,
                requestUnlockLockedUtxos: requestUnlockLockedUtxos,
                toggleLockState: toggleLockState
            )
        }
    }
}

private struct TransactionDetailsFooterActions: View {
    let detailsExpanded: Bool
    let openExplorer: () -> Void
    let toggleDetails: () -> Void

    var body: some View {
        VStack(spacing: 24) {
            Button(action: openExplorer) {
                Text("View in Explorer")
                    .frame(maxWidth: .infinity)
                    .padding()
                    .background(Color.midnightBtn)
                    .foregroundColor(.white)
                    .cornerRadius(10)
                    .padding(.horizontal, detailsExpandedPadding)
            }

            Button(action: toggleDetails) {
                Text(detailsExpanded ? "Hide Details" : "Show Details")
                    .font(.footnote)
                    .fontWeight(.bold)
                    .foregroundStyle(Color.secondary.opacity(0.8))
            }
            .padding(.top, 10)
            .offset(y: -20)
        }
    }
}

private struct TransactionDetailsBackground: View {
    let colorScheme: ColorScheme
    let currentOffset: Double

    var body: some View {
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
    }
}

private struct TransactionLockErrorModifier: ViewModifier {
    @Binding var error: String?

    func body(content: Content) -> some View {
        content
            .alert(
                "Unable to Update Lock",
                isPresented: Binding(
                    get: { error != nil },
                    set: { if !$0 { error = nil } }
                )
            ) {
                Button("OK", role: .cancel) {
                    error = nil
                }
            } message: {
                Text(error ?? "")
            }
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
