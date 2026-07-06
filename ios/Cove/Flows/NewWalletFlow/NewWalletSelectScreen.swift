//
//  NewWalletSelectScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI
import UniformTypeIdentifiers

struct NewWalletSelectScreen: View {
    @Environment(\.sizeCategory) private var sizeCategory
    @Environment(AppManager.self) var app
    @Environment(\.dismiss) private var dismiss

    @Environment(\.colorScheme) var colorScheme
    @State var showSelectDialog: Bool = false

    // private
    @State private var nfcCalled: Bool = false
    let routeFactory: RouteFactory = .init()

    // file import
    @State private var alert: AlertItem? = nil
    @State private var isImporting = false

    /// sheets
    @State private var sheetState: TaggedItem<SheetState>? = nil

    @ViewBuilder
    private func SheetContent(_ state: TaggedItem<SheetState>) -> some View {
        switch state.item {
        case .nfcHelp: NfcHelpView()
        }
    }

    var body: some View {
        GeometryReader { proxy in
            let scrollableLayout = usesCompactLayout(
                sizeCategory: sizeCategory,
                availableHeight: proxy.size.height
            )

            Group {
                if scrollableLayout {
                    ScrollView {
                        mainContent(usesFlexibleTopSpacer: false)
                            .frame(minHeight: proxy.size.height, maxHeight: .infinity, alignment: .bottom)
                    }
                    .scrollIndicators(.hidden)
                } else {
                    bottomActionLayout()
                        .frame(width: proxy.size.width, height: proxy.size.height)
                }
            }
        }
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
                    type: .error(error.localizedDescription)
                )
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
        .sheet(item: $sheetState, content: SheetContent)
        .background(
            Image(.newWalletPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(height: screenHeight * 0.75, alignment: .topTrailing)
                .frame(maxWidth: .infinity)
                .brightness(0.05)
        )
        .background(Color.midnightBlue)
        .navigationTitle("Add New Wallet")
        .navigationBarTitleDisplayMode(.inline)
        .toolbarColorScheme(.dark, for: .navigationBar)
        .toolbar {
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

    private func mainContent(usesFlexibleTopSpacer: Bool) -> some View {
        VStack(spacing: 28) {
            if usesFlexibleTopSpacer {
                Spacer()
            }

            promptContent

            Divider()
                .overlay(.coveLightGray.opacity(0.50))

            walletTypeActions
        }
        .padding()
        .frame(maxHeight: .infinity)
    }

    private func bottomActionLayout() -> some View {
        VStack(spacing: 0) {
            Spacer(minLength: 0)

            promptContent
                .padding(.horizontal)
                .padding(.bottom, 28)

            VStack(spacing: 28) {
                Divider()
                    .overlay(.coveLightGray.opacity(0.50))

                walletTypeActions
            }
            .padding(.horizontal)
            .padding(.top, 16)
            .padding(.bottom, 56)
        }
    }

    private var promptContent: some View {
        VStack(spacing: 28) {
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
        }
    }

    private var walletTypeActions: some View {
        VStack(spacing: 16) {
            HStack(spacing: 14) {
                hardwareWalletButton
                hotWalletButton
            }
            .confirmationDialog(
                "Import hardware wallet using",
                isPresented: $showSelectDialog,
                titleVisibility: .visible
            ) {
                NewWalletSelectConfirmationDialogContent(
                    qrRoute: routeFactory.qrImport(),
                    importFile: { isImporting = true },
                    scanNfc: scanNfc,
                    pasteWallet: pasteWallet
                )
            }

            Button {
                app.pushRoute(RouteFactory().keyTeleportReceive())
            } label: {
                HStack {
                    Image(systemName: "arrow.down.left.and.arrow.up.right")
                        .font(.subheadline)

                    Text("Key Teleport")
                        .font(.subheadline)
                        .fontWeight(.medium)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 16)
                .background(Color.btnPrimary.opacity(0.9))
                .foregroundColor(.midnightBlue)
                .cornerRadius(10)
            }
            .buttonStyle(PlainButtonStyle())

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
    }

    private var hardwareWalletButton: some View {
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
    }

    private var hotWalletButton: some View {
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

    private func newWalletFromXpub(_ xpub: String) {
        do {
            let wallet = try Wallet.newFromXpub(xpub: xpub)
            let id = wallet.id()
            Log.debug("Imported Wallet: \(id)")
            try app.selectWalletOrThrow(id)
            app.alertState = TaggedItem(.importedSuccessfully)
        } catch {
            alert = AlertItem(type: .error(error.localizedDescription))
        }
    }

    private func scanNfc() {
        app.nfcReader.scan()

        DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) {
            withAnimation {
                nfcCalled = true
            }
        }
    }

    private func pasteWallet() {
        let text = UIPasteboard.general.string ?? ""
        if text.isEmpty {
            alert = AlertItem(
                type: .error("No text found on the clipboard.")
            )
            return
        }

        newWalletFromXpub(text)
    }
}

private struct NewWalletSelectConfirmationDialogContent: View {
    let qrRoute: Route
    let importFile: () -> Void
    let scanNfc: () -> Void
    let pasteWallet: () -> Void

    var body: some View {
        NavigationLink(value: qrRoute) {
            Text("QR Code")
        }

        Button("File", action: importFile)
        Button("NFC", action: scanNfc)
        Button("Paste", action: pasteWallet)
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
