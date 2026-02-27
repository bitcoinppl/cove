//
//  CoveApp.swift
//  Cove
//
//  Created by Praveen Perera  on 6/17/24.
//

@_exported import CoveCore
import MijickPopups
import SwiftUI

extension EnvironmentValues {
    @Entry var navigate: (Route) -> Void = { _ in }
}

struct SafeAreaInsetsKey: EnvironmentKey {
    static var defaultValue: EdgeInsets {
        #if os(iOS) || os(tvOS)
            let window = (UIApplication.shared.connectedScenes.first as? UIWindowScene)?.keyWindow
            guard let insets = window?.safeAreaInsets else {
                return EdgeInsets()
            }
            return EdgeInsets(
                top: insets.top, leading: insets.left, bottom: insets.bottom, trailing: insets.right
            )
        #else
            return EdgeInsets()
        #endif
    }
}

public extension EnvironmentValues {
    var safeAreaInsets: EdgeInsets {
        self[SafeAreaInsetsKey.self]
    }
}

@main
struct CoveApp: App {
    @UIApplicationDelegateAdaptor(CoveAppDelegate.self) var appDelegate
    @State private var app: AppManager?
    @State private var auth: AuthManager?
    @State private var bootstrapError: String?
    @State private var bdkMigrationWarning: String?

    init() {
        _ = Keychain(keychain: KeychainAccessor())
        _ = Device(device: DeviceAccesor())
    }

    var body: some Scene {
        WindowGroup {
            Group {
                if let app, let auth {
                    CoveMainView(app: app, auth: auth)
                } else {
                    CoverView(errorMessage: bootstrapError)
                }
            }
            .task {
                do {
                    let warning = try await bootstrap()
                    self.app = AppManager.shared
                    await self.app?.rust.initData()
                    self.app?.asyncRuntimeReady = true
                    self.auth = AuthManager.shared
                    self.bdkMigrationWarning = warning
                } catch {
                    bootstrapError = error.localizedDescription
                }
            }
            .alert(
                "Encryption Migration Issue",
                isPresented: Binding(
                    get: { bdkMigrationWarning != nil },
                    set: { if !$0 { bdkMigrationWarning = nil } }
                )
            ) {
                Button("OK") { bdkMigrationWarning = nil }
            } message: {
                Text(
                    "Some wallet databases couldn't be encrypted. Your wallets still work and encryption will retry on next launch.\n\nIf this persists, please contact feedback@covebitcoinwallet.com"
                )
            }
        }
    }
}
