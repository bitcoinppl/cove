//
//  EnterAddressView.swift
//  Cove
//
//  Created by Praveen Perera on 11/19/24.
//
import SwiftUI

// MARK: Aliases

private typealias FocusField = SendFlowPresenter.FocusField

struct EnterAddressView: View {
    @Environment(SendFlowPresenter.self) private var presenter

    /// args
    @Binding var address: String

    /// private
    @FocusState private var focusField: SendFlowPresenter.FocusField?
    @State private var showingWalletPicker: Bool = false
    @State private var selectedWallet: WalletMetadata?
    @State private var showRawAddress: Bool = false

    var body: some View {
        VStack(spacing: 8) {
            HStack {
                Text("Enter address")
                    .font(.headline)
                    .fontWeight(.bold)

                Spacer()

                Button(action: { showingWalletPicker = true }) {
                    Image(systemName: "wallet.bifold")
                }
                .foregroundStyle(.secondary)

                Button(action: { presenter.sheetState = TaggedItem(.qr) }) {
                    Image(systemName: "qrcode")
                }
                .foregroundStyle(.secondary)
            }
            .id(FocusField.address)

            HStack {
                Text("Where do you want to send to?")
                    .font(.footnote)
                    .foregroundStyle(.secondary.opacity(0.80))

                Spacer()
            }

            if let dest = selectedWallet {
                VStack(alignment: .leading) {
                    HStack {
                        Image(systemName: "wallet.bifold")
                        Text(dest.name).font(.headline).fontWeight(.semibold)
                        Spacer()
                        Button(action: {
                            selectedWallet = nil
                            address = ""
                        }) {
                            Image(systemName: "xmark.circle.fill")
                                .foregroundColor(.secondary)
                        }
                    }
                    .padding()
                    .background(Color.secondary.opacity(0.2))
                    .cornerRadius(8)

                    if showRawAddress {
                        Text(address)
                            .font(.caption)
                            .foregroundColor(.secondary)
                    } else {
                        Button("Show address") {
                            showRawAddress = true
                        }
                        .font(.caption)
                        .foregroundColor(.blue)
                    }
                }
            } else {
                HStack {
                    AddressTextEditor(text: $address)
                        .focused($focusField, equals: .address)
                        .foregroundStyle(.primary.opacity(0.9))
                        .autocorrectionDisabled(true)
                        .keyboardType(.asciiCapable)
                }
            }
        }
        .contentShape(Rectangle())
        .sheet(isPresented: $showingWalletPicker) {
            NavigationView {
                List {
                    let wallets = (try? Database().wallets().allSortedActive()) ?? []
                    ForEach(wallets.filter { $0.id != presenter.manager.id }, id: \.id) { wallet in
                        Button(action: {
                            Task {
                                if let wm = try? WalletManager(id: wallet.id),
                                   let addrInfo = try? await wm.firstAddress()
                                {
                                    address = addrInfo.addressUnformatted()
                                    selectedWallet = wallet
                                    showRawAddress = false
                                    showingWalletPicker = false
                                }
                            }
                        }) {
                            HStack {
                                Text(wallet.name)
                                Spacer()
                            }
                        }
                    }
                }
                .navigationTitle("Select Wallet")
                .navigationBarItems(trailing: Button("Cancel") { showingWalletPicker = false })
            }
        }
        .onTapGesture {
            presenter.focusField = .address
        }
        .onChange(of: presenter.focusField, initial: true) { _, new in focusField = new }
        .onChange(of: focusField, initial: false) { _, new in
            guard let new else { return }
            presenter.focusField = new
        }
        .onChange(of: address, initial: true) { _, new in
            let noSpaces = new.replacingOccurrences(of: " ", with: "").trimmingCharacters(
                in: .whitespaces
            )

            address = noSpaces
        }
        .padding(.top, 14)
    }
}

#Preview {
    AsyncPreview {
        let app = AppManager.shared
        let manager = WalletManager(preview: "preview_only")
        let presenter = SendFlowPresenter(app: app, manager: manager)

        EnterAddressView(address: Binding.constant("bc1qdgxdn046v8tvxtx2k6ml7q7mcanj6dy63atva9"))
            .environment(app)
            .environment(manager)
            .environment(presenter)
            .padding()
    }
}
