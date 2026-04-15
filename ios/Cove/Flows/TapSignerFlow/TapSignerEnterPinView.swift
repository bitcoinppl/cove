//
//  TapSignerEnterPinView.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import SwiftUI

struct TapSignerEnterPin: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner
    let action: AfterPinAction

    var message: String {
        action.userMessage() + " (6-32 characters)"
    }

    // private
    @State private var pin: String = ""
    @FocusState private var isFocused

    /// confirmed pin is correct, now run the action
    func runAction(_ nfc: TapSignerNFC, _ pin: String) {
        switch action {
        case .derive: deriveAction(nfc, pin)
        case .change:
            manager.navigate(
                to:
                .newPin(
                    TapSignerNewPinArgs(
                        tapSigner: tapSigner,
                        startingPin: pin,
                        chainCode: .none,
                        action: .change
                    )
                )
            )
        case .backup:
            backupAction(nfc, pin)
        case let .sign(psbt):
            signAction(nfc, psbt, pin)
        }
    }

    func deriveAction(_ nfc: TapSignerNFC, _ pin: String) {
        Task {
            switch await nfc.derive(pin: pin) {
            case let .success(deriveInfo):
                manager.resetRoute(to: .importSuccess(tapSigner, deriveInfo))
            case let .failure(error):
                if error.isAuthError() {
                    app.sheetState = nil
                    app.alertState = .init(.tapSignerWrongPin(tapSigner: tapSigner, action: .derive))
                } else {
                    app.alertState = .init(.tapSignerDeriveFailed(message: error.description))
                }
            }

            await MainActor.run { self.pin = "" }
        }
    }

    func backupAction(_ nfc: TapSignerNFC, _ pin: String) {
        Task {
            switch await nfc.backup(pin: pin) {
            case let .success(backup):
                let _ = app.saveTapSignerBackup(tapSigner, backup)
                await MainActor.run {
                    self.pin = ""
                    app.sheetState = .none

                    // use imperative ShareSheet for automatic share after NFC read
                    ShareSheet.present(
                        data: hexEncode(bytes: backup),
                        filename: "\(tapSigner.identFileNamePrefix())_backup.txt"
                    ) { _ in }
                }

            case let .failure(error):
                if error.isAuthError() {
                    app.sheetState = nil
                    app.alertState = .init(.tapSignerWrongPin(tapSigner: tapSigner, action: .backup))
                } else {
                    app.alertState = .init(
                        .general(title: "Backup Failed!", message: error.description)
                    )
                }

                await MainActor.run { self.pin = "" }
            }
        }
    }

    func signAction(_ nfc: TapSignerNFC, _ psbt: Psbt, _ pin: String) {
        Task {
            switch await nfc.sign(psbt: psbt, pin: pin) {
            case let .success(signedPsbt):
                do {
                    let db = Database().unsignedTransactions()
                    let txId = psbt.txId()
                    let record = try db.getTxThrow(txId: txId)
                    let route = RouteFactory()
                        .sendConfirm(
                            id: record.walletId(),
                            details: record.confirmDetails(),
                            signedPsbt: signedPsbt
                        )

                    await MainActor.run {
                        self.pin = ""
                        app.sheetState = .none
                        app.pushRoute(route)
                    }
                } catch {
                    await MainActor.run {
                        app.alertState = .init(
                            .general(title: "Error", message: error.localizedDescription)
                        )

                        self.pin = ""
                        app.sheetState = .none
                    }
                }
            case let .failure(error):
                if error.isAuthError() {
                    app.sheetState = nil
                    app.alertState = .init(.tapSignerWrongPin(tapSigner: tapSigner, action: .sign(psbt)))
                } else {
                    app.alertState = .init(
                        .general(title: "Signing Failed!", message: error.description)
                    )
                    app.sheetState = .none
                }

                await MainActor.run { self.pin = "" }
            }
        }
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 40) {
                VStack {
                    HStack {
                        Button(action: { app.sheetState = .none }) {
                            Text("Cancel")
                        }

                        Spacer()
                    }
                    .padding(.top, 20)
                    .padding(.horizontal, 10)
                    .foregroundStyle(.primary)
                    .fontWeight(.semibold)

                    Image(systemName: "lock")
                        .font(.system(size: 100))
                        .foregroundColor(.blue)
                        .padding(.top, 22)
                }

                VStack(spacing: 20) {
                    Text("Enter PIN")
                        .font(.largeTitle)
                        .fontWeight(.bold)

                    Text(message)
                        .font(.subheadline)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.horizontal)

                let columns = Array(repeating: GridItem(.flexible(), spacing: 12, alignment: .center), count: 6)
                LazyVGrid(columns: columns, alignment: .center, spacing: 12) {
                    ForEach(0 ..< min(max(pin.count, 6), 32), id: \.self) { index in
                        Circle()
                            .stroke(.primary, lineWidth: 1.3)
                            .background(
                                Circle()
                                    .fill(pin.count <= index ? Color.clear : .primary)
                            )
                            .frame(width: 18, height: 18)
                            .id(index)
                    }
                }
                .padding(.horizontal, 36)
                .frame(maxWidth: .infinity, alignment: .center)
                .contentShape(Rectangle())
                .onTapGesture { isFocused = true }

                Text("\(pin.count)/32 characters")
                    .font(.caption)
                    .foregroundStyle(.gray)

                TextField("Hidden Input", text: $pin)
                    .opacity(0)
                    .frame(width: 0, height: 0)
                    .focused($isFocused)
                    .keyboardType(.numberPad)

                Button(action: {
                    let nfc = manager.getOrCreateNfc(tapSigner)
                    manager.enteredPin = pin
                    runAction(nfc, pin)
                }) {
                    Text("Continue")
                        .frame(maxWidth: .infinity)
                        .padding()
                        .background(pin.count >= 6 ? Color.blue : Color.gray)
                        .foregroundColor(.white)
                        .cornerRadius(10)
                }
                .disabled(pin.count < 6)
                .padding(.horizontal)

                Spacer()
            }
            .onAppear {
                pin = ""
                isFocused = true
            }
            .onChange(of: isFocused) { _, _ in isFocused = true }
            .onChange(of: pin) { _, newPin in
                if newPin.count > 32 {
                    pin = String(newPin.prefix(32))
                }
            }
        }
        .scrollIndicators(.hidden)
        .navigationBarHidden(true)
    }
}

#Preview {
    TapSignerContainer(
        route: .enterPin(
            tapSigner: tapSignerPreviewNew(preview: true),
            action: .derive
        )
    )
    .environment(AppManager.shared)
    .environment(AuthManager.shared)
}
