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
    @State private var nfcReader = NFCReader()
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
                self.walletOptionButton(
                    title: "On This Device",
                    icon: "iphone",
                    color: .blue,
                    destination: RouteFactory().newHotWallet()
                )

                Button(action: { self.showSelectDialog = true }) {
                    HStack {
                        Image(systemName: "externaldrive")
                            .font(.title2)
                        Text("On a Hardware Wallet")
                            .font(.headline)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 25)
                    .background(.green.opacity(self.colorScheme == .dark ? 0.85 : 1))
                    .foregroundColor(.white)
                    .cornerRadius(12)
                }
                .buttonStyle(PlainButtonStyle())
            }
            .padding(.horizontal)
            .confirmationDialog(
                "Import hardware wallet using",
                isPresented: self.$showSelectDialog,
                titleVisibility: .visible
            ) {
                NavigationLink(value: self.routeFactory.qrImport()) {
                    Text("QR Code")
                }
                Button("File") {
                    self.isImporting = true
                }
                Button("NFC") {
                    self.nfcReader.scan()
                }
            }

            Spacer()
            Spacer()
        }
        .navigationBarTitleDisplayMode(.inline)
        .fileImporter(isPresented: self.$isImporting, allowedContentTypes: [.plainText, .json]) { result in
            switch result {
            case let .success(file):
                do {
                    let fileContents = try readFile(from: file)
                    self.newWalletFromXpub(fileContents)
                } catch {
                    self.alert = AlertItem(type: .error(error.localizedDescription))
                }
            case let .failure(error):
                self.alert = AlertItem(type: .error(error.localizedDescription))
            }
        }
        .alert(item: self.$alert) { alert in
            Alert(
                title: Text(alert.type.title),
                message: Text(alert.type.message),
                dismissButton: .default(Text("OK")) {
                    self.presentationMode.wrappedValue.dismiss()
                }
            )
        }
        .onChange(of: self.nfcReader.scannedMessage) { _, message in
            if let message = message { self.newWalletFromXpub(message) }
        }
    }

    private func newWalletFromXpub(_ xpub: String) {
        do {
            let wallet = try Wallet.newFromXpub(xpub: xpub)
            let id = wallet.id()
            Log.debug("Imported Wallet: \(id)")
            self.alert = AlertItem(type: .success("Imported Wallet Successfully"))
            try self.app.rust.selectWallet(id: id)
        } catch {
            self.alert = AlertItem(type: .error(error.localizedDescription))
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
            .background(color.opacity(self.colorScheme == .dark ? 0.85 : 1))
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
        .environment(MainViewModel())
}
