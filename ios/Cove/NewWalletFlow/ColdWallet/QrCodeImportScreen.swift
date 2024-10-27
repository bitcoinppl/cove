//
//  QrCodeImportScreen.swift
//  Cove
//
//  Created by Praveen Perera on 9/22/24.
//

import SwiftUI

private struct AlertItem: Identifiable {
    let id = UUID()
    let type: AlertType
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

private enum AlertType: Equatable {
    case success(String)
    case error(String)
    case custom(CustomAlert)

    init(_ alert: Alert) {
        self = .custom(CustomAlert(alert))
    }

    var alert: Alert {
        switch self {
        case let .success(message):
            return .init(
                title: Text("Success"), message: Text(message), dismissButton: .default(Text("OK"))
            )
        case let .error(message):
            return .init(
                title: Text("Error"), message: Text(message), dismissButton: .default(Text("OK"))
            )
        case let .custom(alert):
            return alert.alert
        }
    }
}

struct QrCodeImportScreen: View {
    @State private var multiQr: MultiQr?
    @State private var scannedCode: IdentifiableString?
    @State private var showingHelp = false
    @Environment(MainViewModel.self) var app
    @Environment(\.presentationMode) var presentationMode

    // private
    @State private var scanComplete = false
    @State private var totalParts: Int? = nil
    @State private var partsLeft: Int? = nil
    @State private var alert: AlertItem? = nil

    private let screenHeight = UIScreen.main.bounds.height

    var partsScanned: Int {
        if let totalParts, let partsLeft {
            totalParts - partsLeft
        } else {
            0
        }
    }

    var qrCodeHeight: CGFloat {
        screenHeight * 0.6
    }

    var body: some View {
        VStack {
            if !scanComplete {
                VStack {
                    ZStack {
                        RoundedRectangle(cornerRadius: 20)
                            .stroke(Color.primary, lineWidth: 3)
                            .frame(height: qrCodeHeight)

                        ScannerView(
                            codeTypes: [.qr],
                            scanMode: .oncePerCode,
                            scanInterval: 0.1,
                            showAlert: false,
                            completion: handleScan
                        )
                        .frame(height: qrCodeHeight)
                        .clipShape(RoundedRectangle(cornerRadius: 18))
                    }
                    .padding(.horizontal)

                    if let totalParts, let partsLeft {
                        Text("Scanned \(partsScanned) of \(totalParts)")
                            .font(.subheadline)
                            .fontWeight(.medium)
                            .padding(.top, 8)

                        Text("\(partsLeft) parts left")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fontWeight(.bold)
                    }
                }
                .padding(.top, 18)
                .padding(.bottom, 36)
            }

            Button(action: {
                showingHelp = true
            }) {
                Label("Help", systemImage: "questionmark.circle.fill")
                    .padding()
                    .frame(minWidth: 120)
            }
            .buttonStyle(.borderedProminent)
            .tint(.blue)
            .sheet(isPresented: $showingHelp) {
                HelpView()
            }
            .padding()
        }
        .padding()
        .alert(item: $alert) { alert in
            alert.type.alert
        }
        .onChange(of: scannedCode) { _, scannedCode in
            guard let scannedCode = scannedCode else { return }
            do {
                let wallet = try Wallet.newFromXpub(xpub: scannedCode.value)
                let id = wallet.id()
                Log.debug("Imported Wallet: \(id)")
                self.alert = AlertItem(type: .success("Imported Wallet Successfully"))
                try app.rust.selectWallet(id: id)
            } catch let WalletError.WalletAlreadyExists(id) {
                self.alert = AlertItem(type: .success("Wallet already exists: \(id)"))
                if (try? app.rust.selectWallet(id: id)) == nil {
                    self.alert = AlertItem(type: .error("Unable to select wallet"))
                }
            } catch {
                self.alert = AlertItem(type: .error(error.localizedDescription))
            }
        }
        .navigationTitle("Scan QR")
    }

    private func handleScan(result: Result<ScanResult, ScanError>) {
        switch result {
        case let .success(result):
            guard case let .string(stringValue) = result.data else { return }

            if multiQr == nil {
                multiQr = MultiQr.newFromString(qr: stringValue)
                totalParts = Int(multiQr?.totalParts() ?? 0)
            }

            guard let multiQr else { return }

            // single QR
            if multiQr.isSingle() {
                scanComplete = true
                scannedCode = IdentifiableString(stringValue)
                return
            }

            // BBQr
            do {
                let result = try multiQr.addPart(qr: stringValue)
                partsLeft = Int(result.partsLeft())

                if result.isComplete() {
                    scanComplete = true
                    let data = try result.finalResult()
                    scannedCode = IdentifiableString(data)
                }
            } catch {
                Log.error("error scanning bbqr part: \(error)")
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

struct HelpView: View {
    var body: some View {
        Text("How do get my wallet export QR code?")
            .font(.title)
            .fontWeight(.bold)
            .multilineTextAlignment(.center)
            .padding(.horizontal, 12)
            .frame(alignment: .center)
            .padding(.vertical, 18)

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
        .padding(22)
    }
}

#Preview {
    QrCodeImportScreen()
        .environment(MainViewModel())
}

#Preview("help") {
    HelpView()
}
