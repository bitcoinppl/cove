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

    var body: some View {
        WalletManagerHost(walletId: id, loading: {
            WalletSettingsLoadingOrError(error: error, metadata: app.walletMetadata(id: id)) {
                app.trySelectLatestOrNewWallet()
            }
        }, onError: { error in
            self.error = "Failed to get wallet \(error.localizedDescription)"
            Log.error(self.error!)
        }) { manager in
            WalletSettingsRouteView(
                manager: manager,
                route: route,
                walletName: walletNameBinding(manager)
            )
        }
    }
}

private struct WalletSettingsRouteView: View {
    private let content: AnyView

    init(manager: WalletManager, route: WalletSettingsRoute, walletName: Binding<String>) {
        content = switch route {
        case .main:
            AnyView(WalletSettingsView(manager: manager))
        case .changeName:
            AnyView(WalletSettingsChangeNameView(name: walletName))
        }
    }

    var body: some View {
        content
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
            WalletSettingsLoadingInformationSection(metadata: metadata)
            WalletSettingsLoadingSettingsSection(
                metadata: metadata,
                colorColumns: colorColumns
            )
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

private struct WalletSettingsLoadingInformationSection: View {
    let metadata: WalletMetadata

    var body: some View {
        Section(header: Text("Wallet Information")) {
            WalletSettingsLoadingRow(title: "Network", value: metadata.network.description)
            WalletSettingsLoadingRow(title: "Wallet Type", value: String(metadata.walletType))
        }
    }
}

private struct WalletSettingsLoadingSettingsSection: View {
    let metadata: WalletMetadata
    let colorColumns: [GridItem]

    var body: some View {
        Section(header: Text("Settings")) {
            WalletSettingsLoadingNameRow(name: metadata.name)
            WalletSettingsLoadingColorPicker(
                metadata: metadata,
                colorColumns: colorColumns
            )
            Toggle(isOn: .constant(metadata.showLabels)) {
                Text("Show transaction labels")
                    .font(.subheadline)
            }
            .disabled(true)
        }
    }
}

private struct WalletSettingsLoadingNameRow: View {
    let name: String

    var body: some View {
        HStack {
            Text("Name")
            Spacer()
            Text(name)
                .font(.subheadline)
                .foregroundColor(.secondary)
            Image(systemName: "chevron.right")
                .foregroundColor(Color(UIColor.tertiaryLabel))
                .font(.footnote)
                .fontWeight(.semibold)
        }
        .font(.subheadline)
    }
}

private struct WalletSettingsLoadingColorPicker: View {
    let metadata: WalletMetadata
    let colorColumns: [GridItem]

    var body: some View {
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
                        WalletSettingsLoadingColor(
                            color: color,
                            isSelected: color == metadata.color
                        )
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                .frame(maxWidth: .infinity)
            }
        }
        .padding(.vertical, 8)
    }
}

private struct WalletSettingsLoadingColor: View {
    let color: WalletColor
    let isSelected: Bool

    var body: some View {
        ZStack {
            if isSelected {
                Circle()
                    .stroke(Color(color).opacity(0.7), lineWidth: 2)
                    .frame(width: 32, height: 32)
            }
            Circle()
                .fill(Color(color))
                .frame(width: 28, height: 28)
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
