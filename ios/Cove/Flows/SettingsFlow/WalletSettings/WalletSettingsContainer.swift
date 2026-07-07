//
//  WalletSettingsContainer.swift
//  Cove
//
//  Created by Praveen Perera on 12/5/24.
//

import Foundation
import SwiftUI

struct WalletSettingsContainer: View {
    @Environment(AppManager.self) var app

    // args
    let id: WalletId
    let route: WalletSettingsRoute

    /// private
    @State private var error: String? = nil

    func walletNameBinding(_ manager: WalletManager) -> Binding<String> {
        Binding(
            get: { manager.walletMetadata.name },
            set: { manager.dispatch(action: .updateName($0)) }
        )
    }

    @ViewBuilder
    func WalletSettingsRoute(manager: WalletManager, route: WalletSettingsRoute) -> some View {
        switch route {
        case .main:
            WalletSettingsView(manager: manager)
        case .changeName:
            WalletSettingsChangeNameView(name: walletNameBinding(manager))
        }
    }

    var body: some View {
        WalletManagerHost(walletId: id, loading: {
            WalletSettingsLoadingOrError(error: error, metadata: app.walletMetadata(id: id)) {
                app.trySelectLatestOrNewWallet()
            }
        }, onError: { error in
            self.error = "Failed to get wallet \(error.localizedDescription)"
            Log.error(self.error!)
        }) { manager in
            WalletSettingsRoute(manager: manager, route: route)
        }
    }
}

private struct WalletSettingsLoadingOrError: View {
    let error: String?
    let metadata: WalletMetadata?
    let recover: () -> Void

    var body: some View {
        Group {
            if let error {
                Text(error)
            } else if let metadata {
                WalletSettingsLoadingView(metadata: metadata)
            } else {
                FullPageLoadingView()
            }
        }
        .task {
            guard let error else { return }
            Log.error(error)
            try? await Task.sleep(for: .seconds(5))
            recover()
        }
    }
}

private struct WalletSettingsLoadingView: View {
    let metadata: WalletMetadata

    private let colorColumns = Array(repeating: GridItem(.flexible(), spacing: 0), count: 5)

    var body: some View {
        List {
            Section(header: Text("Wallet Information")) {
                WalletSettingsLoadingRow(title: "Network", value: metadata.network.description)
                WalletSettingsLoadingRow(title: "Wallet Type", value: String(metadata.walletType))
            }

            Section(header: Text("Settings")) {
                HStack {
                    Text("Name")
                    Spacer()

                    Text(metadata.name)
                        .font(.subheadline)
                        .foregroundColor(.secondary)

                    Image(systemName: "chevron.right")
                        .foregroundColor(Color(UIColor.tertiaryLabel))
                        .font(.footnote)
                        .fontWeight(.semibold)
                }
                .font(.subheadline)

                VStack(spacing: 14) {
                    HStack {
                        Text("Wallet Color")
                            .font(.subheadline)
                        Spacer()
                    }

                    HStack {
                        Rectangle()
                            .fill(metadata.swiftColor)
                            .cornerRadius(10)
                            .frame(width: 80, height: 80)

                        LazyVGrid(columns: colorColumns, spacing: 20) {
                            ForEach(defaultWalletColors(), id: \.self) { color in
                                ZStack {
                                    if color == metadata.color {
                                        Circle()
                                            .stroke(Color(color).opacity(0.7), lineWidth: 2)
                                            .frame(width: 32, height: 32)
                                    }

                                    Circle()
                                        .fill(Color(color))
                                        .frame(width: 28, height: 28)
                                }
                            }
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                        }
                        .frame(maxWidth: .infinity)
                    }
                }
                .padding(.vertical, 8)

                Toggle(isOn: .constant(metadata.showLabels)) {
                    Text("Show transaction labels")
                        .font(.subheadline)
                }
                .disabled(true)
            }
        }
        .navigationTitle(metadata.name)
        .overlay {
            ProgressView()
                .progressViewStyle(.circular)
                .controlSize(.large)
                .frame(width: 72, height: 72)
                .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 8))
        }
    }
}

private struct WalletSettingsLoadingRow: View {
    let title: LocalizedStringKey
    let value: String

    var body: some View {
        HStack {
            Text(title)
            Spacer()
            Text(value)
                .foregroundColor(.secondary)
        }
        .font(.subheadline)
    }
}
