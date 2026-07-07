import SwiftUI

private struct WalletManagerHostResolutionKey: Hashable {
    let walletId: WalletId
    let managerId: ObjectIdentifier?
}

struct WalletManagerHost<Loading: View, Content: View>: View {
    @Environment(AppManager.self) private var app

    let walletId: WalletId
    let loading: () -> Loading
    let onError: (Error) -> Void
    let content: (WalletManager) -> Content

    init(
        walletId: WalletId,
        @ViewBuilder loading: @escaping () -> Loading,
        onError: @escaping (Error) -> Void = { _ in },
        @ViewBuilder content: @escaping (WalletManager) -> Content
    ) {
        self.walletId = walletId
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
            _ = try await app.ensureWalletManagerLoaded(id: walletId)
        } catch is CancellationError {
            return
        } catch {
            guard !Task.isCancelled else { return }
            onError(error)
        }
    }
}
