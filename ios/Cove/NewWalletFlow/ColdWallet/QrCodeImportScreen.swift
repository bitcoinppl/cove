//
//  QrCodeImportScreen.swift
//  Cove
//
//  Created by Praveen Perera on 9/22/24.
//

import CodeScanner
import SwiftUI

struct IdentifiableString: Identifiable, Equatable {
    let id = UUID()
    let value: String
}

private struct AlertItem: Identifiable {
    let id = UUID()
    let type: AlertType
}

private enum AlertType: Equatable {
    case success(String)
    case error(String)

    var message: String {
        switch self {
        case let .success(message): return message
        case let .error(message): return message
        }
    }

    var title: String {
        switch self {
        case .success: return "Success"
        case .error: return "Error"
        }
    }
}

struct QrCodeImportScreen: View {
    @State private var multiQr: MultiQr?
    @State private var scannedCode: IdentifiableString?
    @State private var showingHelp = false
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
        screenHeight * 0.4
    }

    var body: some View {
        VStack {
            Text("Scan your wallet export QR code")
                .font(.title)
                .fontWeight(.bold)
                .multilineTextAlignment(.center)
                .padding()

            if !scanComplete {
                VStack {
                    ZStack {
                        RoundedRectangle(cornerRadius: 20)
                            .stroke(Color.primary, lineWidth: 2)
                            .frame(height: qrCodeHeight)

                        CodeScannerView(codeTypes: [.qr],
                                        scanMode: .oncePerCode,
                                        scanInterval: 0.20,
                                        simulatedData: "Simulated QR Code",
                                        completion: handleScan)
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
                .padding(.vertical, 36)
            }

            Button("Where do I get the QR code?") {
                showingHelp = true
            }
            .padding()
            .background(Color.blue)
            .foregroundColor(.white)
            .cornerRadius(10)
            .sheet(isPresented: $showingHelp) {
                HelpView()
            }
            .padding()
        }
        .padding()
        .alert(item: $alert) { alert in
            Alert(
                title: Text(alert.type.title),
                message: Text(alert.type.message),
                dismissButton: .default(Text("OK")) {
                    presentationMode.wrappedValue.dismiss()
                }
            )
        }
        .onChange(of: scannedCode) { _, scannedCode in
            guard let scannedCode = scannedCode else { return }
            do {
                _ = try Wallet.newFromXpub(xpub: scannedCode.value)
                self.alert = AlertItem(type: .success("Imported Wallet Successfully"))
            } catch {
                self.alert = AlertItem(type: .error(error.localizedDescription))
            }
        }
        .navigationTitle("Scan QR")
    }

    func handleScan(result: Result<ScanResult, ScanError>) {
        switch result {
        case let .success(result):
            if multiQr == nil {
                multiQr = MultiQr(qr: result.string)
                totalParts = Int(multiQr?.totalParts() ?? 0)
            }

            guard let multiQr else { return }
            if multiQr.isSingle() {
                scanComplete = true
                scannedCode = IdentifiableString(value: result.string)
            }

            do {
                let result = try multiQr.addPart(qr: result.string)
                partsLeft = Int(result.partsLeft())

                if result.isComplete() {
                    scanComplete = true
                    let data = try result.finalResult()
                    scannedCode = IdentifiableString(value: data)
                }
            } catch {
                print("error scanning bbqr part: \(error)")
            }

        case let .failure(error):
            print("Scanning failed: \(error.localizedDescription)")
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

        VStack(alignment: .leading, spacing: 32) {
            VStack(alignment: .leading, spacing: 12) {
                Text("On ColdCard Q1")
                    .font(.title2)
                    .fontWeight(.bold)

                Text("1. Go to 'Advanced / Tools'")
                Text("2. Export Wallet > Generic JSON")
                Text("3. Press the 'Enter' button, then the 'QR' button")
                Text("5. Scan the Generated QR code")
            }

            Divider()

            VStack(alignment: .leading, spacing: 12) {
                Text("On Other Hardware Wallets")
                    .font(.title2)
                    .fontWeight(.bold)

                Text("1. In your hardware wallet, go to settings")
                Text("2. Look for 'Export'")
                Text("3. Select 'Generic JSON', 'Sparrow', 'Electrum', and many other formats should also work")
                Text("4. Generate QR code")
                Text("5. Scan the Generated QR code")
            }
        }
        .padding(22)
    }
}

#Preview {
    QrCodeImportScreen()
}

#Preview("help") {
    HelpView()
}
