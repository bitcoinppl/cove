//
//  MainSettingsSections.swift
//  Cove
//
//  Created by Praveen Perera on 6/30/26.
//

import SwiftUI

struct MainSettingsGeneralSection: View {
    var body: some View {
        Section(header: Text("General")) {
            SettingsRow(title: "Network", route: .network, symbol: "network")
            SettingsRow(title: "Appearance", route: .appearance, symbol: "sun.max.fill")
            SettingsRow(
                title: "Node",
                route: .node,
                symbol: "point.3.filled.connected.trianglepath.dotted"
            )
            SettingsRow(
                title: "Block Explorer",
                route: .blockExplorer,
                symbol: "safari"
            )
            SettingsRow(title: "Currency", route: .fiatCurrency, symbol: "dollarsign.circle")
        }
    }
}

struct MainSettingsBackupSection: View {
    let isVisible: Bool
    let exportAll: () -> Void
    let importAll: () -> Void
    let verifyBackup: () -> Void

    var body: some View {
        if isVisible {
            Section(header: backupHeader) {
                SettingsRow(title: "Export All", symbol: "square.and.arrow.up") {
                    exportAll()
                }

                SettingsRow(title: "Import All", symbol: "square.and.arrow.down") {
                    importAll()
                }

                SettingsRow(title: "Verify Backup", symbol: "checkmark.shield") {
                    verifyBackup()
                }
            }
        }
    }

    private var backupHeader: some View {
        MainSettingsBetaHeader(title: "Backup")
    }
}

struct MainSettingsBetaHeader: View {
    let title: String

    var body: some View {
        HStack(spacing: 6) {
            Text(title)

            Text("BETA")
                .font(.caption2)
                .fontWeight(.semibold)
                .foregroundStyle(.white)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Color.statusWarning, in: Capsule())
        }
    }
}

/// The Payjoin switch, shown only when this build's Payjoin rollout offers it
struct MainSettingsPayjoinSection: View {
    @State private var setting = Database().globalConfig().payjoinSetting()

    var body: some View {
        if case let .available(enabled, beta) = setting {
            Section {
                Toggle("Payjoin", isOn: enabledBinding(enabled))
            } header: {
                header(beta: beta)
            } footer: {
                Text(
                    "Lets a receiver that supports Payjoin add a coin to your payment, for more privacy. If they don't, Cove sends it as a normal transaction."
                )
            }
        }
    }

    @ViewBuilder
    private func header(beta: Bool) -> some View {
        if beta {
            MainSettingsBetaHeader(title: "Payments")
        } else {
            Text("Payments")
        }
    }

    private func enabledBinding(_ enabled: Bool) -> Binding<Bool> {
        Binding(
            get: { enabled },
            set: { newValue in
                do {
                    try Database().globalConfig().setPayjoinEnabled(enabled: newValue)
                } catch {
                    Log.error("Failed to save the Payjoin choice: \(error)")
                }

                // read back what was saved, so a failed save leaves the switch where it was
                setting = Database().globalConfig().payjoinSetting()
            }
        )
    }
}

struct MainSettingsBetaToggleSection: View {
    let isVisible: Bool
    let betaToggle: Binding<Bool>
    let betaImportExportToggle: Binding<Bool>

    var body: some View {
        if isVisible {
            Section {
                Toggle("Beta Features", isOn: betaToggle)
                Toggle("Enable Beta Import Export", isOn: betaImportExportToggle)
            } footer: {
                Text("Disable to hide experimental features")
            }
        }
    }
}
