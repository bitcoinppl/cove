//
//  Settings2.swift
//  Cove
//
//  Created by Praveen Perera on 7/10/24.
//
// TODO: for reference delete later

import SwiftUI

struct SettingsView2: View {
    var body: some View {
        NavigationView {
            List {
                Section {
                    SettingsRow(icon: "person.circle.fill", iconColor: .blue, text: "Apple ID")
                }

                Section {
                    SettingsRow(icon: "airplane", iconColor: .orange, text: "Airplane Mode")
                    SettingsRow(icon: "wifi", iconColor: .blue, text: "Wi-Fi")
                    SettingsRow(icon: "antenna.radiowaves.left.and.right", iconColor: .green, text: "Cellular")
                }

                Section {
                    SettingsRow(icon: "bell.fill", iconColor: .red, text: "Notifications")
                    SettingsRow(icon: "speaker.wave.3.fill", iconColor: .pink, text: "Sounds & Haptics")
                    SettingsRow(icon: "moon.fill", iconColor: .indigo, text: "Focus")
                    SettingsRow(icon: "hourglass", iconColor: .indigo, text: "Screen Time")
                }
            }
            .listStyle(InsetGroupedListStyle())
            .navigationTitle("Settings")
        }
        .enableInjection()
    }

    #if DEBUG
    @ObserveInjection var forceRedraw
    #endif
}

struct SettingsRow: View {
    let icon: String
    let iconColor: Color
    let text: String

    var body: some View {
        HStack {
            Image(systemName: icon)
                .foregroundColor(.white)
                .frame(width: 29, height: 29)
                .background(iconColor)
                .cornerRadius(6)

            Text(text)
        }
        .enableInjection()
    }

    #if DEBUG
    @ObserveInjection var forceRedraw
    #endif
}

#Preview {
    SettingsView2()
}
