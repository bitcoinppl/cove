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
    @Environment(\.dismiss) private var dismiss

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
                    .background(
                        .blue.opacity(colorScheme == .dark ? 0.85 : 1)
                    )
                    .foregroundColor(.white)
                    .cornerRadius(12)
                }
                .buttonStyle(PlainButtonStyle())

                Button(action: { showSelectDialog = true }) {
                    HStack {
                        Image(systemName: "externaldrive")
                            .font(.title2)
                        Text("On a Hardware Wallet")
                            .font(.headline)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 22)
                    .background(
                        .green.opacity(colorScheme == .dark ? 0.85 : 1)
                    )
                    .foregroundColor(.white)
                    .cornerRadius(12)
                }
                .buttonStyle(PlainButtonStyle())

                Divider()

                HStack {
                    Button(action: app.nfcReader.scan) {
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

                    Button(action: app.scanQr) {
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
                isPresented: $showSelectDialog,
                titleVisibility: .visible
            ) {
                NavigationLink(value: routeFactory.qrImport()) {
                    Text("QR Code")
                }
                Button("File") {
                    isImporting = true
                }
                Button("NFC") {
                    nfcReader.scan()

                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) {
                        withAnimation {
                            nfcCalled = true
                        }
                    }
                }
                Button("Paste") {
                    let text = UIPasteboard.general.string ?? ""
                    if text.isEmpty {
                        alert = AlertItem(
                            type: .error("No text found on the clipboard."))
                        return
                    }

                    newWalletFromXpub(text)
                }
            }

            Spacer()

            if nfcCalled {
                Button(action: {
                    sheetState = TaggedItem(.nfcHelp)
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
        .fileImporter(
            isPresented: $isImporting,
            allowedContentTypes: [.plainText, .json]
        ) { result in
            do {
                let file = try result.get()
                let fileContents = try FileReader(for: file).read()
                newWalletFromXpub(fileContents)
            } catch {
                alert = AlertItem(
                    type: .error(error.localizedDescription))
            }
        }
        .alert(item: $alert) { alert in
            Alert(
                title: Text(alert.type.title),
                message: Text(alert.type.message),
                dismissButton: .default(Text("OK")) {
                    dismiss()
                }
            )
        }
        .onChange(of: nfcReader.scannedMessage) { _, message in
            if let message { newWalletFromXpub(message) }
        }
        .sheet(item: $sheetState, content: SheetContent)
    }

    private func newWalletFromXpub(_ xpub: String) {
        do {
            let wallet = try Wallet.newFromXpub(xpub: xpub)
            let id = wallet.id()
            Log.debug("Imported Wallet: \(id)")
            alert = AlertItem(
                type: .success("Imported Wallet Successfully"))
            try app.rust.selectWallet(id: id)
        } catch {
            alert = AlertItem(type: .error(error.localizedDescription))
        }
    }
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
        case let .success(message): message
        case let .error(message): message
        }
    }

    var title: String {
        switch self {
        case .success: "Success"
        case .error: "Error"
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