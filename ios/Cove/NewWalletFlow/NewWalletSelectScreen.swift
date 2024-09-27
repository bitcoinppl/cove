//
//  NewWalletSelectScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI
import UniformTypeIdentifiers

struct NewWalletSelectScreen: View {
    @Environment(MainViewModel.self) var app
    @Environment(\.presentationMode) var presentationMode

    @Environment(\.colorScheme) var colorScheme
    @State var showSelectDialog: Bool = false

    // private
    let routeFactory: RouteFactory = .init()

    // file import
    @State private var alert: AlertItem? = nil
    @State private var isImporting = false

    var body: some View {
        VStack(spacing: 30) {
            Text("How do you want to secure your Bitcoin?")
                .font(.largeTitle)
                .fontWeight(.bold)
                .padding(.top)

            Spacer()

            VStack(spacing: 30) {
                walletOptionButton(
                    title: "On This Device",
                    icon: "iphone",
                    color: .blue,
                    destination: RouteFactory().newHotWallet()
                )

                Button(action: { showSelectDialog = true }) {
                    HStack {
                        Image(systemName: "externaldrive")
                            .font(.title2)
                        Text("On a Hardware Wallet")
                            .font(.headline)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 25)
                    .background(.green.opacity(colorScheme == .dark ? 0.85 : 1))
                    .foregroundColor(.white)
                    .cornerRadius(12)
                }
                .buttonStyle(PlainButtonStyle())
            }
            .padding(.horizontal)
            .confirmationDialog(
                "Import hardware wallet using",
                isPresented: $showSelectDialog,
                titleVisibility: .visible
            ) {
                NavigationLink(value: routeFactory.qrImport()) {
                    Text("QR Code")
                }
                Button("File") {
                    isImporting = true
                }
                NavigationLink(value: routeFactory.nfcImport()) {
                    Text("NFC coming soon...")
                }
            }

            Spacer()
            Spacer()
        }
        .navigationBarTitleDisplayMode(.inline)
        .fileImporter(isPresented: $isImporting, allowedContentTypes: [.plainText, .json]) { result in
            switch result {
            case let .success(file):
                do {
                    let fileContents = try readFile(from: file)
                    let wallet = try Wallet.newFromXpub(xpub: fileContents)
                    let id = wallet.id()
                    Log.debug("Imported Wallet: \(id)")
                    self.alert = AlertItem(type: .success("Imported Wallet Successfully"))
                    try app.rust.selectWallet(id: id)
                } catch {
                    self.alert = AlertItem(type: .error(error.localizedDescription))
                }
            case let .failure(error):
                self.alert = AlertItem(type: .error(error.localizedDescription))
            }
        }
        .alert(item: $alert) { alert in
            Alert(
                title: Text(alert.type.title),
                message: Text(alert.type.message),
                dismissButton: .default(Text("OK")) {
                    presentationMode.wrappedValue.dismiss()
                }
            )
        }
    }

    private func walletOptionButton(
        title: String, icon: String, color: Color, destination: some Hashable
    ) -> some View {
        NavigationLink(value: destination) {
            HStack {
                Image(systemName: icon)
                    .font(.title2)
                Text(title)
                    .font(.headline)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 25)
            .background(color.opacity(colorScheme == .dark ? 0.85 : 1))
            .foregroundColor(.white)
            .cornerRadius(12)
        }
        .buttonStyle(PlainButtonStyle())
    }

    func readFile(from url: URL) throws -> String {
        guard url.startAccessingSecurityScopedResource() else {
            throw FileReadError(message: "Failed to access the file at \(url.path)")
        }

        defer { url.stopAccessingSecurityScopedResource() }
        return try String(contentsOf: url, encoding: .utf8)
    }
}

private struct FileReadError: Error {
    let message: String
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

#Preview {
    NewWalletSelectScreen()
}
