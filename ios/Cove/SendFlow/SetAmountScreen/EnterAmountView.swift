//
//  SendFlowEnterAmountView.swift
//  Cove
//
//  Created by Praveen Perera on 11/19/24.
//
import SwiftUI

private typealias FocusField = SendFlowSetAmountFocusField

struct EnterAmountView: View {
    let model: WalletViewModel

    @Binding var sendAmount: String
    @FocusState var focusField: SendFlowSetAmountFocusField?
    let sendAmountFiat: String

    // private
    @State private var showingMenu: Bool = false

    var metadata: WalletMetadata { model.walletMetadata }

    var body: some View {
        VStack(spacing: 8) {
            HStack(alignment: .bottom) {
                TextField("", text: $sendAmount)
                    .focused($focusField, equals: .amount)
                    .multilineTextAlignment(.center)
                    .font(.system(size: 48, weight: .bold))
                    .keyboardType(
                        metadata.selectedUnit == .btc ? .decimalPad : .numberPad
                    )
                    .offset(
                        x: metadata.selectedUnit == .btc
                            ? screenWidth * 0.10 : screenWidth * 0.11
                    )
                    .padding(.horizontal, 30)
                    .minimumScaleFactor(0.01)
                    .lineLimit(1)
                    .scrollDisabled(true)

                HStack(spacing: 0) {
                    Button(action: { showingMenu.toggle() }) {
                        Text(model.unit)
                            .padding(.vertical, 10)

                        Image(systemName: "chevron.down")
                            .font(.caption)
                            .fontWeight(.bold)
                            .padding(.top, 2)
                            .padding(.leading, 4)
                    }
                    .foregroundStyle(.primary)
                }
                .popover(isPresented: $showingMenu) {
                    VStack(alignment: .center, spacing: 0) {
                        Button("sats") {
                            model.dispatch(action: .updateUnit(.sat))
                            showingMenu = false
                        }
                        .padding(12)
                        .buttonStyle(.plain)

                        Divider()

                        Button("btc") {
                            model.dispatch(action: .updateUnit(.btc))
                            showingMenu = false
                        }
                        .padding(12)
                        .buttonStyle(.plain)
                    }
                    .padding(.vertical, 8)
                    .padding(.horizontal, 12)
                    .frame(minWidth: 120, maxWidth: 200)
                    .presentationCompactAdaptation(.popover)
                    .foregroundStyle(.primary.opacity(0.8))
                }
            }

            Text(sendAmountFiat)
                .font(.title3)
                .foregroundColor(.secondary)
        }

        .padding(.vertical, 4)
    }
}
