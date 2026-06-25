import SwiftUI

struct MainSettingsSecuritySection: View {
    let canUseBiometrics: Bool
    let toggleBiometric: Binding<Bool>
    let togglePin: Binding<Bool>
    let toggleWipeMePin: Binding<Bool>
    let toggleDecoyPin: Binding<Bool>
    let onChangePin: () -> Void

    var body: some View {
        Section("Security") {
            if canUseBiometrics {
                SettingsToggle(title: "Enable FaceID", symbol: "faceid", item: toggleBiometric)
            }

            SettingsToggle(title: "Enable PIN", symbol: "lock", item: togglePin)

            if togglePin.wrappedValue {
                SettingsRow(title: "Change PIN", symbol: "lock.open.rotation") {
                    onChangePin()
                }
                .foregroundStyle(.link)

                SettingsToggle(
                    title: "Enable Wipe Data PIN",
                    symbol: "exclamationmark.lock.fill",
                    item: toggleWipeMePin
                )

                SettingsToggle(
                    title: "Enable Decoy PIN",
                    symbol: "theatermasks",
                    item: toggleDecoyPin
                )
            }
        }
    }
}
