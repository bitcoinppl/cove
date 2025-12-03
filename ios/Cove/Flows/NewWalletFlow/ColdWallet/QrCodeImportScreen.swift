//
//  QrCodeImportScreen.swift
//  Cove
//
//  Created by Praveen Perera on 9/22/24.
//

import SwiftUI
import UIKit

private struct AlertItem: Identifiable {
    let id = UUID()
    let type: AlertType

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.id == rhs.id
    }
}

private struct CustomAlert: Equatable, Identifiable {
    let id = UUID()
    let alert: Alert

    init(_ alert: Alert) {
        self.alert = alert
    }

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.id == rhs.id
    }
}

private enum AlertType {
    case success(String, String = "Success", () -> Void = {})
    case error(String, String = "Error", () -> Void = {})
    case custom(CustomAlert)

    init(_ alert: Alert) {
        self = .custom(CustomAlert(alert))
    }

    var alert: Alert {
        switch self {
        case let .success(message, title, action):
            .init(
                title: Text(title),
                message: Text(message),
                dismissButton: .cancel(Text("OK"), action: action)
            )
        case let .error(message, title, action):
            .init(
                title: Text(title),
                message: Text(message),
                dismissButton: .cancel(Text("OK"), action: action)
            )
        case let .custom(alert):
            alert.alert
        }
    }
}

struct QrCodeImportScreen: View {
    @State private var scanner = QrScanner()
    @State private var scannedMultiFormat: TaggedItem<MultiFormat>?
    @State private var showingHelp = false
    @Environment(AppManager.self) var app
    @Environment(\.dismiss) private var dismiss

    // private
    @State private var scanComplete = false
    @State private var progress: ScanProgress? = nil
    @State private var alert: AlertItem? = nil

    private let screenHeight = UIScreen.main.bounds.height

    var qrCodeHeight: CGFloat {
        screenHeight * 0.6
    }

    var body: some View {
        VStack {
            if !scanComplete {
                ZStack {
                    ScannerView(
                        codeTypes: [.qr],
                        scanMode: .oncePerCode,
                        scanInterval: 0.1,
                        showAlert: false,
                        completion: handleScan
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .ignoresSafeArea(.all)

                    VStack {
                        Spacer()

                        Text("Scan Wallet Export QR Code")
                            .font(.title2)
                            .foregroundStyle(.white)
                            .fontWeight(.semibold)

                        Spacer()
                        Spacer()
                        Spacer()
                        Spacer()
                        Spacer()

                        if let progress {
                            VStack(spacing: 8) {
                                Text(progress.displayText())
                                    .font(.subheadline)
                                    .fontWeight(.medium)
                                    .padding(.top, 8)

                                if let detailText = progress.detailText() {
                                    Text(detailText)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                        .fontWeight(.bold)
                                }
                            }
                            .foregroundStyle(.white)
                        }

                        Spacer()
                    }
                }
            }
        }
        .alert(item: $alert) { alert in
            alert.type.alert
        }
        .onChange(of: scannedMultiFormat) { _, scannedMultiFormat in
            guard let multiFormat = scannedMultiFormat?.item else { return }
            do {
                let wallet: Wallet
                switch multiFormat {
                case let .hardwareExport(export):
                    wallet = try Wallet.newFromExport(export: export)
                default:
                    Log.warn("Unexpected format for wallet import: \(multiFormat)")
                    alert = AlertItem(type: .error("Unexpected format for wallet import"))
                    // reset state so user can retry
                    scanComplete = false
                    self.scannedMultiFormat = nil
                    scanner.reset()
                    return
                }
                let id = wallet.id()
                Log.debug("Imported Wallet: \(id)")
                alert = AlertItem(type: .success("Imported Wallet Successfully"))
                try app.rust.selectWallet(id: id)
            } catch let WalletError.MultiFormat(error) {
                app.popRoute()
                self.alert = AlertItem(type: .error(error.description, "Invalid Format"))
            } catch let WalletError.WalletAlreadyExists(id) {
                self.alert = AlertItem(type: .success("Wallet already exists: \(id)"))
                if (try? app.rust.selectWallet(id: id)) == nil {
                    app.popRoute()
                    self.alert = AlertItem(type: .error("Unable to select wallet"))
                }
            } catch {
                Log.warn("Error importing hardware wallet: \(error)")
                alert = AlertItem(type: .error(error.localizedDescription))
            }
        }
        .toolbar {
            ToolbarItem(placement: .navigationBarTrailing) {
                Button("?") {
                    showingHelp = true
                }
                .buttonStyle(.plain)
                .sheet(isPresented: $showingHelp) {
                    HelpView()
                }
                .foregroundStyle(.white)
                .fontWeight(.medium)
                .padding()
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .ignoresSafeArea(.all)
    }

    private func triggerProgressHaptic() {
        let generator = UIImpactFeedbackGenerator(style: .light)
        generator.impactOccurred()
    }

    private func triggerSuccessHaptic() {
        let generator = UINotificationFeedbackGenerator()
        generator.notificationOccurred(.success)
    }

    private func triggerHaptic(_ haptic: HapticFeedback) {
        switch haptic {
        case .progress:
            triggerProgressHaptic()
        case .success:
            triggerSuccessHaptic()
        case .none:
            break
        }
    }

    private func handleScan(result: Result<ScanResult, ScanError>) {
        switch result {
        case let .success(scanResult):
            let qr = StringOrData(scanResult.data)

            do {
                switch try scanner.scan(qr: qr) {
                case let .complete(data, haptic):
                    triggerHaptic(haptic)
                    scanComplete = true
                    scannedMultiFormat = TaggedItem(data)
                    scanner.reset()

                case let .inProgress(prog, haptic):
                    triggerHaptic(haptic)
                    progress = prog
                }
            } catch {
                scanner.reset()
                app.alertState = TaggedItem(
                    .general(
                        title: "QR Scan Error",
                        message: "Unable to scan QR code, error: \(error.localizedDescription)"
                    )
                )
            }

        case let .failure(error):
            if case ScanError.permissionDenied = error {
                DispatchQueue.main.async {
                    let customAlert =
                        Alert(
                            title: Text("Camera Access Required"),
                            message: Text(
                                "Please allow camera access in Settings to use this feature."),
                            primaryButton: Alert.Button.default(Text("Settings")) {
                                DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                                    app.popRoute()
                                }

                                let url = URL(string: UIApplication.openSettingsURLString)!
                                UIApplication.shared.open(url)
                            },
                            secondaryButton: Alert.Button.cancel {
                                Task {
                                    await MainActor.run {
                                        app.popRoute()
                                    }
                                }
                            }
                        )

                    alert = AlertItem(type: .init(customAlert))
                }
            }
        }
    }
}

private struct HelpView: View {
    var body: some View {
        VStack(spacing: 24) {
            Text("How do I get my wallet export QR code?")
                .font(.title)
                .fontWeight(.bold)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 12)
                .frame(alignment: .center)
                .padding(.top, 12)
                .foregroundStyle(.primary)

            ScrollView {
                VStack(alignment: .leading, spacing: 32) {
                    VStack(alignment: .leading, spacing: 12) {
                        Text("ColdCard Q1")
                            .font(.title2)
                            .fontWeight(.bold)

                        Text("1. Go to 'Advanced / Tools'")
                        Text("2. Export Wallet > Generic JSON")
                        Text("3. Press the 'Enter' button, then the 'QR' button")
                        Text("4. Scan the Generated QR code")
                    }

                    Divider()

                    VStack(alignment: .leading, spacing: 12) {
                        Text("ColdCard MK3/MK4")
                            .font(.title2)
                            .fontWeight(.bold)

                        Text("1. Go to 'Advanced / Tools'")
                        Text("2. Export Wallet > Descriptor")
                        Text("3. Press the Enter (✓) and select your wallet type")
                        Text("4. Scan the Generated QR code")
                    }

                    Divider()

                    VStack(alignment: .leading, spacing: 12) {
                        Text("Sparrow Desktop")
                            .font(.title2)
                            .fontWeight(.bold)

                        Text("1. Click on Settings, in the left side bar")
                        Text("2. Click on 'Export...' button at the bottom")
                        Text("3. Under 'Output Descriptor' click the 'Show...' button")
                        Text("4. Make sure 'Show BBQr' is selected")
                        Text("5. Scan the generated QR code")
                    }

                    Divider()

                    VStack(alignment: .leading, spacing: 12) {
                        Text("Other Hardware Wallets")
                            .font(.title2)
                            .fontWeight(.bold)

                        Text("1. In your hardware wallet, go to settings")
                        Text("2. Look for 'Export'")
                        Text(
                            "3. Select 'Generic JSON', 'Sparrow', 'Electrum', and many other formats should also work"
                        )
                        Text("4. Generate QR code")
                        Text("5. Scan the Generated QR code")
                    }
                }
            }
        }
        .scrollIndicators(.hidden)
        .foregroundColor(.primary)
        .fontWeight(.regular)
        .padding()
    }
}

#Preview {
    QrCodeImportScreen()
        .environment(AppManager.shared)
        .background(.red)
}

#Preview("HelpView") {
    HelpView()
}
