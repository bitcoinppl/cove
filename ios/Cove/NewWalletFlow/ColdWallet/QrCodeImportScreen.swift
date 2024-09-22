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

    var body: some View {
        VStack {
            Text("Scan your wallet export QR code")
                .font(.headline)
                .padding()

            ZStack {
                RoundedRectangle(cornerRadius: 20)
                    .stroke(Color.blue, lineWidth: 3)
                    .frame(height: 300)

                CodeScannerView(codeTypes: [.qr],
                                simulatedData: "Simulated QR Code",
                                completion: handleScan)
                    .frame(height: screenHeight * 0.3)
                    .clipShape(RoundedRectangle(cornerRadius: 18))
            }
            .padding(.horizontal)

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
        VStack {
            Text("How to get your wallet export QR code")
                .font(.headline)
                .padding()

            Text("1. Open your wallet app\n2. Go to settings\n3. Look for 'Export' or 'Backup'\n4. Generate QR code\n5. Scan the generated QR code with this app")
                .padding()

            Spacer()
        }
    }
}

#Preview {
    QrCodeImportScreen()
}
