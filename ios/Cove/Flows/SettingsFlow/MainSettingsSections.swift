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

struct MainSettingsAdvancedSection: View {
    var body: some View {
        Section(header: Text("Advanced")) {
            SettingsRow(
                title: "PayJoin Relay",
                route: .ohttpRelay,
                symbol: "arrow.triangle.2.circlepath"
            )
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
        HStack(spacing: 6) {
            Text("Backup")

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
