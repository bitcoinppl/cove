//
//  TransactionDetailsView.swift
//  Cove
//
//  Created by Praveen Perera on 9/17/24.
//

import SwiftUI

struct TransactionDetailsView: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.openURL) private var openURL
    @Environment(\.sizeCategory) var sizeCategory

    @State private var numberOfConfirmations: Int? = nil
    @State private var scrollPosition = ScrollPosition()

    @State private var initialOffset: Double? = nil
    @State private var currentOffset: Double = 0
    @State private var lockState: TransactionLockState? = nil
    @State private var isUpdatingLockState = false
    @State private var lockStateError: String? = nil
    @State private var lockStateLoadError: String? = nil

    // public
    let id: WalletId
    let txId: TxId
    private let initialDetails: TransactionDetails
    let refreshOnAppear: Bool
    var manager: WalletManager

    /// read from cache (observable), fallback to initial details
    var transactionDetails: TransactionDetails {
        manager.transactionDetails[txId] ?? initialDetails
    }

    init(
        id: WalletId,
        txId: TxId,
        transactionDetails: TransactionDetails,
        refreshOnAppear: Bool = true,
        manager: WalletManager
    ) {
        self.id = id
        self.txId = txId
        self.initialDetails = transactionDetails
        self.refreshOnAppear = refreshOnAppear
        self.manager = manager
    }

    var headerIcon: HeaderIcon {
        HeaderIcon(
            isSent: transactionDetails.isSent(),
            isConfirmed: transactionDetails.isConfirmed(),
            numberOfConfirmations: numberOfConfirmations
        )
    }

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    var detailsExpanded: Bool {
        metadata.detailsExpanded
    }

    @ViewBuilder
    var ReceivedDetails: some View {
        VStack {
            headerIcon

            VStack(spacing: 4) {
                Text(
                    transactionDetails.isConfirmed()
                        ? "Transaction Received" : "Transaction Pending"
                )
                .font(.title)
                .fontWeight(.semibold)
                .padding(.top, 8)

                // add, edit, remove label
                TransactionDetailsLabelView(details: transactionDetails, manager: manager)
            }
        }

        // confirmed
        if transactionDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction was successfully received")
                    .foregroundColor(.secondary)

                Text(transactionDetails.confirmationDateTime() ?? "Unknown")
                    .fontWeight(.semibold)
                    .foregroundColor(.secondary)
            }
            .multilineTextAlignment(.center)
        }

        // pending
        if !transactionDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction is pending. ")
                    .foregroundColor(.secondary)

                Text("Please check back soon for an update.")
                    .foregroundColor(.secondary)
            }
            .multilineTextAlignment(.center)
        }

        VStack(spacing: 8) {
            Text(transactionDetails.displayAmount(metadata: metadata))
                .font(.largeTitle)
                .fontWeight(.bold)
                .padding(.top, 12)

            AsyncView(
                cachedValue: transactionDetails.amountFiatFmtCached(),
                operation: transactionDetails.amountFiatFmt
            ) { amount in
                Text(amount).foregroundStyle(.primary.opacity(0.8))
            }
        }

        Group {
            if transactionDetails.isConfirmed() {
                TransactionCapsule(text: "Received", icon: "arrow.down.left", color: .statusSuccess)
            } else {
                TransactionCapsule(
                    text: "Receiving", icon: "arrow.down.left",
                    color: .coolGray, textColor: .black.opacity(0.8)
                )
            }
        }
        .padding(.top, 12)

        TransactionLockControl

        // confirmations pills
        if let confirmations = numberOfConfirmations, confirmations < 3 {
            VStack {
                Divider().padding(.vertical, 18)
                ConfirmationIndicatorView(current: confirmations)
            }
            .padding(.horizontal, detailsExpandedPadding)
        }

        // MARK: Received Details Expanded

        if metadata.detailsExpanded {
            ReceivedDetailsExpandedView(
                manager: manager, transactionDetails: transactionDetails,
                numberOfConfirmations: numberOfConfirmations
            )
        }
    }

    @ViewBuilder
    var SentDetails: some View {
        VStack {
            headerIcon

            VStack(spacing: 4) {
                Text(transactionDetails.isConfirmed() ? "Transaction Sent" : "Transaction Pending")
                    .font(.title)
                    .fontWeight(.semibold)
                    .padding(.top, 6)

                // add, edit, remove label
                TransactionDetailsLabelView(details: transactionDetails, manager: manager)
            }
        }

        // confirmed
        if transactionDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction was sent on")
                    .foregroundColor(.secondary)

                Text(transactionDetails.confirmationDateTime() ?? "Unknown")
                    .fontWeight(.semibold)
                    .foregroundColor(.secondary)
            }
            .multilineTextAlignment(.center)
        }

        // pending
        if !transactionDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction is pending. ")
                    .foregroundColor(.secondary)

                Text("Please check back soon for an update.")
                    .fontWeight(.semibold)
                    .foregroundColor(.secondary)
            }
            .multilineTextAlignment(.center)
        }

        VStack(spacing: 8) {
            Text(transactionDetails.displayAmount(metadata: metadata))
                .font(.largeTitle)
                .fontWeight(.bold)
                .padding(.top, 12)

            AsyncView(
                cachedValue: transactionDetails.amountFiatFmtCached(),
                operation: transactionDetails.amountFiatFmt
            ) { amount in
                Text(amount).foregroundStyle(.primary.opacity(0.8))
            }
        }

        Group {
            if transactionDetails.isConfirmed() {
                TransactionCapsule(
                    text: "Sent", icon: "arrow.up.right",
                    color: .black, textColor: .white
                )
            } else {
                TransactionCapsule(
                    text: "Sending", icon: "arrow.up.right",
                    color: .coolGray, textColor: .black.opacity(0.8)
                )
            }
        }
        .padding(.top, 12)

        TransactionLockControl

        if let confirmations = numberOfConfirmations, confirmations < 3 {
            VStack {
                Divider().padding(.vertical, 18)
                ConfirmationIndicatorView(current: confirmations)
            }
            .padding(.horizontal, detailsExpandedPadding)
        }

        if metadata.detailsExpanded {
            SentDetailsExpandedView(
                manager: manager, transactionDetails: transactionDetails,
                numberOfConfirmations: numberOfConfirmations
            )
        }
    }

    @ViewBuilder
    var TransactionLockControl: some View {
        if lockStateLoadError != nil {
            transactionLockControlContent(
                title: String(localized: "Unable to load lock state"),
                buttonTitle: String(localized: "Retry"),
                systemImage: "arrow.clockwise",
                action: { Task { await refreshTransactionLockState() } }
            )
        } else {
            switch lockState {
            case .some(.none), nil:
                EmptyView()
            case .some(.unlocked), .some(.locked), .some(.mixed):
                transactionLockControlContent(
                    title: lockStateText,
                    buttonTitle: isUpdatingLockState
                        ? String(localized: "Updating...")
                        : lockStateButtonText,
                    systemImage: lockStateButtonIcon,
                    isUpdating: isUpdatingLockState,
                    action: {
                        guard !isUpdatingLockState else { return }

                        isUpdatingLockState = true
                        Task { await toggleTransactionLockState() }
                    }
                )
            }
        }
    }

    func transactionLockControlContent(
        title: String,
        buttonTitle: String,
        systemImage: String,
        isUpdating: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        VStack(spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: systemImage)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)

                Text(title)
                    .font(.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(.secondary)
            }

            Button(action: action) {
                HStack(spacing: 6) {
                    if isUpdating {
                        ProgressView()
                            .controlSize(.mini)
                    } else {
                        Image(systemName: systemImage)
                            .font(.footnote.weight(.semibold))
                    }

                    Text(buttonTitle)
                        .font(.footnote)
                        .fontWeight(.semibold)
                }
                .padding(.vertical, 8)
                .padding(.horizontal, 12)
                .background(Color.systemGray5)
                .foregroundStyle(.primary)
                .clipShape(Capsule())
                .opacity(isUpdating ? 0.72 : 1)
            }
            .buttonStyle(.plain)
            .disabled(isUpdating)
        }
        .padding(.top, 2)
    }

    var lockStateText: String {
        switch lockState {
        case .some(.locked):
            String(localized: "Locked")
        case .some(.mixed):
            String(localized: "Mixed")
        case .some(.unlocked):
            String(localized: "Unlocked")
        case .some(.none), nil:
            ""
        }
    }

    var lockStateButtonText: String {
        switch lockState {
        case .some(.locked):
            String(localized: "Unlock Transaction")
        case .some(.mixed):
            String(localized: "Lock Transaction")
        case .some(.unlocked):
            String(localized: "Lock Transaction")
        case .some(.none), nil:
            ""
        }
    }

    var lockStateButtonIcon: String {
        switch lockState {
        case .some(.locked):
            "lock.open"
        case .some(.mixed), .some(.unlocked):
            "lock"
        case .some(.none), nil:
            "lock"
        }
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

    var body: some View {
        ContentScrollView {
            VStack(spacing: 24) {
                if sizeCategory < .extraExtraExtraLarge || isMiniDevice { Spacer() }

                Group {
                    if transactionDetails.isReceived() {
                        ReceivedDetails
                    } else {
                        SentDetails
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

            // start watcher after a delay to avoid race condition with onDisappear
            if !transactionDetails.isConfirmed() {
                try? await Task.sleep(for: .seconds(2))
                manager.dispatch(action: .startTransactionWatcher(transactionDetails.txId()))
            }

            // continues to check for confirmations
            await updateNumberOfConfirmations()
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
            let details = try await manager.rust.transactionDetails(txId: txId)
            await MainActor.run {
                manager.updateTransactionDetailsCache(txId: txId, details: details)
            }

            // also update confirmations
            if let blockNumber = details.blockNumber() {
                if let confirmations = try? await manager.rust.numberOfConfirmations(blockHeight: blockNumber) {
                    await MainActor.run {
                        withAnimation {
                            self.numberOfConfirmations = Int(confirmations)
                        }
                    }
                }
            }
        } catch {
            Log.error("Error refreshing transaction details: \(error)")
        }
    }

    func refreshTransactionLockState() async {
        do {
            let state = try await manager.transactionLockState(for: initialDetails.txId())
            await MainActor.run {
                withAnimation {
                    lockState = state
                    lockStateLoadError = nil
                }
            }
        } catch {
            Log.error("Error refreshing transaction lock state: \(error)")
            await MainActor.run {
                lockStateLoadError = error.localizedDescription
            }
        }
    }

    func toggleTransactionLockState() async {
        do {
            let state = try await manager.toggleTransactionLockState(for: initialDetails.txId())
            await MainActor.run {
                withAnimation {
                    lockState = state
                }
                isUpdatingLockState = false
            }
        } catch {
            await MainActor.run {
                lockStateError = error.localizedDescription
                isUpdatingLockState = false
            }
        }
    }

    func getAndSetNumberOfConfirmations(from details: TransactionDetails) async -> Int? {
        if let blockNumber = details.blockNumber() {
            let numberOfConfirmations = try? await manager.rust.numberOfConfirmations(
                blockHeight: blockNumber
            )

            guard let numberOfConfirmations else { return nil }

            await MainActor.run {
                withAnimation {
                    self.numberOfConfirmations = Int(numberOfConfirmations)
                }
            }

            return Int(numberOfConfirmations)
        }

        return nil
    }

    func updateNumberOfConfirmations() async {
        var needsFrequentCheck = true
        var errors = 0

        while true {
            Log.debug(
                "checking for number of confirmations for txId: \(txId), currently: \(numberOfConfirmations ?? 0)"
            )

            do {
                // fetch fresh details and update cache
                if let details = try? await manager.rust.transactionDetails(txId: txId) {
                    await MainActor.run {
                        manager.updateTransactionDetailsCache(txId: txId, details: details)
                    }

                    // get confirmations from fresh details
                    let numberOfConfirmations = await getAndSetNumberOfConfirmations(from: details)
                    if let numberOfConfirmations, numberOfConfirmations >= 3, needsFrequentCheck {
                        Log.debug(
                            "transaction fully confirmed with \(numberOfConfirmations) confirmations"
                        )
                        needsFrequentCheck = false
                    }
                }

                if needsFrequentCheck {
                    try await Task.sleep(for: .seconds(30))
                } else {
                    try await Task.sleep(for: .seconds(60))
                }
            } catch let error as CancellationError {
                Log.debug("check for confirmation task cancelled: \(error)")
                break
            } catch {
                Log.error("Error checking for confirmation: \(error)")
                errors = errors + 1
                if errors > 10 { break }
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
        let details = TransactionDetails.previewConfirmedReceived()

        TransactionDetailsView(
            id: WalletId(),
            txId: details.txId(),
            transactionDetails: details,
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}

#Preview("confirmed sent") {
    AsyncPreview {
        let details = TransactionDetails.previewConfirmedSent()

        TransactionDetailsView(
            id: WalletId(),
            txId: details.txId(),
            transactionDetails: details,
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}

#Preview("pending received") {
    AsyncPreview {
        let details = TransactionDetails.previewPendingReceived()

        TransactionDetailsView(
            id: WalletId(),
            txId: details.txId(),
            transactionDetails: details,
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}

#Preview("pending sent") {
    AsyncPreview {
        let details = TransactionDetails.previewPendingSent()

        TransactionDetailsView(
            id: WalletId(),
            txId: details.txId(),
            transactionDetails: details,
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}
