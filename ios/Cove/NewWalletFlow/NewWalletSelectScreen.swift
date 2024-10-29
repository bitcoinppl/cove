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
    @State private var nfcCalled: Bool = false
    let routeFactory: RouteFactory = .init()

    // file import
    @State private var alert: AlertItem? = nil
    @State private var isImporting = false

    // sheets
    @State private var sheetState: TaggedItem<SheetState>? = nil

    @ViewBuilder
    private func SheetContent(_ state: TaggedItem<SheetState>) -> some View {
        switch state.item {
        case .nfcHelp: NfcHelpView()
        }
    }

    var body: some View {
        VStack(spacing: 30) {
            Text("How do you want to secure your Bitcoin?")
                .font(.largeTitle)
                .fontWeight(.bold)
                .padding(.top)

            Spacer()

            VStack(spacing: 30) {
                NavigationLink(value: RouteFactory().newHotWallet()) {
                    HStack {
                        Image(systemName: "iphone")
                            .font(.title2)
                        Text("On This Device")
                            .font(.headline)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 22)
                    .background(.blue.opacity(self.colorScheme == .dark ? 0.85 : 1))
                    .foregroundColor(.white)
                    .cornerRadius(12)
                }
                .buttonStyle(PlainButtonStyle())

                Button(action: { self.showSelectDialog = true }) {
                    HStack {
                        Image(systemName: "externaldrive")
                            .font(.title2)
                        Text("On a Hardware Wallet")
                            .font(.headline)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 22)
                    .background(.green.opacity(self.colorScheme == .dark ? 0.85 : 1))
                    .foregroundColor(.white)
                    .cornerRadius(12)
                }
                .buttonStyle(PlainButtonStyle())

                Divider()

                HStack {
                    Button(action: self.app.nfcReader.scan) {
                        HStack(spacing: 16) {
                            Image(systemName: "wave.3.right")
                                .font(.system(size: 16))
                            Text("NFC")
                                .font(.headline)
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 18)
                        .background(.black.gradient)
                        .foregroundColor(.white)
                        .cornerRadius(12)
                    }
                    .buttonStyle(PlainButtonStyle())

                    Button(action: self.app.scanQr) {
                        HStack(spacing: 16) {
                            Image(systemName: "qrcode")
                                .font(.title2)
                            Text("QR")
                                .font(.headline)
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 18)
                        .background(.black.gradient)
                        .foregroundColor(.white)
                        .cornerRadius(12)
                    }
                    .buttonStyle(PlainButtonStyle())
                }
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

                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) {
                        withAnimation {
                            self.nfcCalled = true
                        }
                    }
                }
            }

            Spacer()

            if self.nfcCalled {
                Button(action: {
                    self.sheetState = TaggedItem(.nfcHelp)
                }) {
                    HStack {
                        Image(systemName: "wave.3.right")
                        Text("NFC Help")
                    }
                }
            }

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
        .sheet(item: self.$sheetState, content: self.SheetContent)
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

private enum SheetState {
    case nfcHelp
}

#Preview {
    NewWalletSelectScreen()
        .environment(MainViewModel())
}
