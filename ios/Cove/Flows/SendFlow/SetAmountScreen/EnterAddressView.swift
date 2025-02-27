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

    // args
    @Binding var address: String

    // private
    @FocusState private var focusField: SendFlowPresenter.FocusField?

    var body: some View {
        VStack(spacing: 8) {
            HStack {
                Text("Enter address")
                    .font(.headline)
                    .fontWeight(.bold)

                Spacer()

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

            HStack {
                AddressTextEditor(text: $address)
                    .focused($focusField, equals: .address)
                    .foregroundStyle(.primary.opacity(0.9))
                    .autocorrectionDisabled(true)
                    .keyboardType(.asciiCapable)
            }
        }
        .onChange(of: presenter.focusField, initial: true) { _, new in focusField = new }
        .onChange(of: focusField, initial: false) { _, new in
            guard let new else { return }
            presenter.focusField = new
        }
        .onChange(of: address, initial: true) { _, new in
            let noSpaces = new.replacingOccurrences(of: " ", with: "").trimmingCharacters(
                in: .whitespaces)
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
