//
//  QrCodeImportScreen.swift
//  Cove
//
//  Created by Praveen Perera on 9/22/24.
//

import CodeScanner
import SwiftUI

struct IdentifiableString: Identifiable {
    let id = UUID()
    let value: String
}

struct QrCodeImportScreen: View {
    @State private var scannedCode: IdentifiableString?
    @State private var showingHelp = false
    @Environment(\.presentationMode) var presentationMode

    // private
    private let screenHeight = UIScreen.main.bounds.height

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

            ZStack {
                RoundedRectangle(cornerRadius: 20)
                    .stroke(Color.primary, lineWidth: 2)
                    .frame(height: qrCodeHeight)

                CodeScannerView(codeTypes: [.qr],
                                scanMode: .continuous,
                                scanInterval: 0.20,
                                simulatedData: "Simulated QR Code",
                                completion: handleScan)
                    .frame(height: qrCodeHeight)
                    .clipShape(RoundedRectangle(cornerRadius: 18))
            }
            .padding(.horizontal)
            .padding(.vertical, 36)

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
        .alert(item: $scannedCode) { code in
            Alert(
                title: Text("Scanned Code"),
                message: Text(code.value),
                dismissButton: .default(Text("OK")) {
                    presentationMode.wrappedValue.dismiss()
                }
            )
        }
        .navigationTitle("Scan QR")
    }

    func handleScan(result: Result<ScanResult, ScanError>) {
        switch result {
        case .success(let result):
            scannedCode = IdentifiableString(value: result.string)
        case .failure(let error):
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
