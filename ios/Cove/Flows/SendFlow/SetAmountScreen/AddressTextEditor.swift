//
//  AddressTextEditor.swift
//  Cove
//
//  Created by Praveen Perera on 11/7/24.
//

import SwiftUI

struct AddressTextEditor: View {
    @Environment(SendFlowPresenter.self) var presenter

    // args
    @Binding var text: String

    // private
    @FocusState private var focusField: SendFlowPresenter.FocusField?

    var isFocused: Bool {
        presenter.focusField == .address
    }

    var textBinding: Binding<String> {
        Binding(
            get: { isFocused ? text : "" },
            set: { newValue in text = newValue }
        )
    }

    var body: some View {
        ZStack(alignment: .topLeading) {
            TextEditor(text: textBinding)
                .focused($focusField, equals: .address)
                .offset(x: -4)
                .font(.footnote)
                .fontDesign(.monospaced)
                .fontWeight(.semibold)

            if !isFocused {
                Text(text.addressSpacedOut())
                    .lineLimit(3)
                    .onTapGesture {
                        presenter.focusField = .address
                    }
                    .font(.footnote)
                    .fontDesign(.monospaced)
                    .fontWeight(.semibold)
            }

            if text.isEmpty {
                Text("bc1p...")
                    .font(.footnote)
                    .fontWeight(.semibold)
                    .foregroundStyle(.tertiary)
                    .padding(.top, 8)
                    .padding(.leading, 5)
            }
        }
        .onChange(of: presenter.focusField, initial: true) { _, new in focusField = new }
        .frame(height: 50)
    }
}

#Preview("focused") {
    AsyncPreview {
        let app = AppManager()
        let manager = WalletManager(preview: "preview_only")
        let presenter = SendFlowPresenter(app: app, manager: manager)

        AddressTextEditor(
            text: .constant("bc1qw8wrek2m7nlqldll66ajnwr9mh64syvkt67zlu")
        )
        .environment(presenter)
        .onAppear {
            presenter.focusField = .address
        }
    }
}

#Preview("not focused") {
    AsyncPreview {
        let app = AppManager()
        let manager = WalletManager(preview: "preview_only")
        let presenter = SendFlowPresenter(app: app, manager: manager)

        AddressTextEditor(
            text: .constant("bc1qwzrryqr3ja8w7hnja2spmkgfdcgvqwp5swz4af4ngsjecfz0w0pqud7k38")
        )
        .environment(presenter)
        .onAppear {
            presenter.focusField = .none
        }
    }
}
