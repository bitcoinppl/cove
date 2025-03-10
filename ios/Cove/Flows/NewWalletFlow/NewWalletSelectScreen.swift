//
//  NewWalletSelectScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI
import UniformTypeIdentifiers

struct NewWalletSelectScreen: View {
    @Environment(AppManager.self) var app
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
        VStack(spacing: 28) {
            Spacer()

            HStack {
                DotMenuView(selected: 0, size: 5)
                Spacer()
            }

            HStack {
                Text("How do you want to secure your Bitcoin?")
                    .font(.system(size: 38, weight: .semibold))
                    .lineSpacing(1.2)
                    .foregroundColor(.white)

                Spacer()
            }

            Divider()
                .overlay(.coveLightGray.opacity(0.50))

            HStack(spacing: 14) {
                Button(action: { showSelectDialog = true }) {
                    HStack {
                        BitcoinShieldIcon(width: 15, color: .midnightBlue)

                        Text("Hardware Wallet")
                            .font(.subheadline)
                            .fontWeight(.medium)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 20)
                    .padding(.horizontal, 10)
                    .background(Color.btnPrimary)
                    .foregroundColor(.midnightBlue)
                    .cornerRadius(10)
                }

                NavigationLink(value: RouteFactory().newHotWallet()) {
                    HStack {
                        Image(systemName: "iphone")
                            .font(.subheadline)
                            .symbolRenderingMode(.monochrome)

                        Text("On This Device")
                            .font(.subheadline)
                            .fontWeight(.medium)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 20)
                    .padding(.horizontal, 10)
                    .background(Color.btnPrimary)
                    .foregroundColor(.midnightBlue)
                    .cornerRadius(10)
                }
                .buttonStyle(PlainButtonStyle())
            }
            .confirmationDialog(
                "Import hardware wallet using",
                isPresented: $showSelectDialog,
                titleVisibility: .visible
            ) {
                ConfirmationDialogContent
            }

            if nfcCalled {
                Button(action: {
                    sheetState = TaggedItem(.nfcHelp)
                }) {
                    HStack {
                        Image(systemName: "wave.3.right")
                        Text("NFC Help")
                            .font(.subheadline)
                    }
                }
                .foregroundColor(.white)
            }
        }
        .padding()
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
        .onChange(of: nfcReader.scannedMessage, initial: false, onChangeScannedMessage)
        .sheet(item: $sheetState, content: SheetContent)
        .frame(maxHeight: .infinity)
        .background(
            Image(.newWalletPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(height: screenHeight * 0.75, alignment: .topTrailing)
                .frame(maxWidth: .infinity)
                .brightness(0.05)
        )
        .background(Color.midnightBlue)
        .toolbar {
            ToolbarItem(placement: .principal) {
                Text("Add New Wallet")
                    .font(.callout)
                    .fontWeight(.semibold)
                    .foregroundStyle(.white)
            }

            ToolbarItemGroup(placement: .topBarTrailing) {
                HStack(spacing: 6) {
                    Button(action: app.scanQr) {
                        Image(systemName: "qrcode")
                            .foregroundColor(.white)
                    }

                    Button(action: app.nfcReader.scan) {
                        Image(systemName: "wave.3.right")
                            .foregroundColor(.white)
                    }
                }
            }
        }
    }

    private func onChangeScannedMessage(_: NfcMessage?, _ message: NfcMessage?) {
        guard let xpub = message?.string() else { return }
        newWalletFromXpub(xpub)
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

    @ViewBuilder
    var ConfirmationDialogContent: some View {
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
    NavigationStack {
        NewWalletSelectScreen()
            .environment(AppManager.shared)
    }
}
