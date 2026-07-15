import SwiftUI

private struct WalletManagerHostResolutionKey: Hashable {
    let walletId: WalletId
    let managerId: ObjectIdentifier?
}

struct WalletManagerHost<Loading: View, Content: View>: View {
    @Environment(AppManager.self) private var app

    let walletId: WalletId
    let preparesWalletRoute: Bool
    let loading: () -> Loading
    let onError: (Error) -> Void
    let content: (WalletManager) -> Content

    init(
        walletId: WalletId,
        preparesWalletRoute: Bool = false,
        @ViewBuilder loading: @escaping () -> Loading,
        onError: @escaping (Error) -> Void = { _ in },
        @ViewBuilder content: @escaping (WalletManager) -> Content
    ) {
        self.walletId = walletId
        self.preparesWalletRoute = preparesWalletRoute
        self.loading = loading
        self.onError = onError
        self.content = content
    }

    private var manager: WalletManager? {
        app.cachedWalletManager(id: walletId)
    }

    private var resolutionKey: WalletManagerHostResolutionKey {
        .init(walletId: walletId, managerId: manager.map(ObjectIdentifier.init))
    }

    var body: some View {
        Group {
            if let manager {
                content(manager)
                    .environment(manager)
            } else {
                loading()
            }
        }
        .task(id: resolutionKey) {
            await resolveManagerIfNeeded()
        }
    }

    @MainActor
    private func resolveManagerIfNeeded() async {
        guard manager == nil else { return }

        do {
            if preparesWalletRoute {
                let generation = app.captureLoadAndResetGeneration()
                _ = try await app.prepareSelectedWallet(id: walletId, generation: generation)
            } else {
                _ = try await app.ensureWalletManagerLoaded(id: walletId)
            }
        } catch is CancellationError {
            return
        } catch {
            guard !Task.isCancelled else { return }
            onError(error)
        }
    }
}
