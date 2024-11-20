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
                Text("Set address")
                    .font(.headline)
                    .fontWeight(.bold)

                Spacer()
            }
            .id(FocusField.address)
            .padding(.top, 10)

            HStack {
                Text("Where do you want to send to?")
                    .font(.callout)
                    .foregroundStyle(.secondary.opacity(0.80))
                    .fontWeight(.medium)
                Spacer()

                Button(action: { presenter.sheetState = TaggedItem(.qr) }) {
                    Image(systemName: "qrcode")
                }
                .foregroundStyle(.secondary)
                .foregroundStyle(.secondary)
            }

            HStack {
                PlaceholderTextEditor(text: $address, placeholder: "bc1q.....")
                    .focused($focusField, equals: .address)
                    .frame(height: 50)
                    .font(.system(size: 16, design: .none))
                    .foregroundStyle(.primary.opacity(0.9))
                    .autocorrectionDisabled(true)
                    .keyboardType(.asciiCapable)
                    .offset(x: -2)
            }
        }
        .onChange(of: presenter.focusField, initial: true) { _, new in focusField = new }
        .padding(.top, 14)
    }
}

#Preview {
    EnterAddressView(address: Binding.constant("bc1qdgxdn046v8tvxtx2k6ml7q7mcanj6dy63atva9"))
        .environment(SendFlowSetAmountPresenter(
            app: MainViewModel(),
            model: WalletViewModel(preview: "preview_only"))
        )
        .padding()
}
