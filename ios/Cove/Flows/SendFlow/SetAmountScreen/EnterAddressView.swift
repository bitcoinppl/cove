//
//  EnterAddressView.swift
//  Cove
//
//  Created by Praveen Perera on 11/19/24.
//
import SwiftUI

// MARK: Aliases

private typealias FocusField = SendFlowSetAmountPresenter.FocusField

struct EnterAddressView: View {
    @Environment(SendFlowSetAmountPresenter.self) private var presenter

    // args
    @Binding var address: String

    // private
    @FocusState private var focusField: SendFlowSetAmountPresenter.FocusField?

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
        .padding(.top, 14)
    }
}

#Preview {
    AsyncPreview {
        let app = MainViewModel()
        let model = WalletViewModel(preview: "preview_only")
        let presenter = SendFlowSetAmountPresenter(app: app, model: model)

        EnterAddressView(address: Binding.constant("bc1qdgxdn046v8tvxtx2k6ml7q7mcanj6dy63atva9"))
            .environment(app)
            .environment(model)
            .environment(presenter)
            .padding()
    }
}
