//
//  SendFlowHeaderView.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//

import Foundation
import SwiftUI

struct SendFlowHeaderView: View {
    @Environment(\.dismiss) private var dismiss

    @Bindable var model: WalletViewModel
    let amount: Amount

    // private
    @State var showingMenu: Bool = false
    private var metadata: WalletMetadata { model.walletMetadata }
    private var balanceString: String {
        if !metadata.sensitiveVisible {
            return "************"
        }

        // btc or sats
        return switch metadata.selectedUnit {
        case .btc: amount.btcString()
        case .sat: amount.satsString()
        }
    }

    var body: some View {
        ZStack {
            // background
            Image(.headerPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(height: 300, alignment: .topTrailing)
                .frame(maxWidth: .infinity)
                .ignoresSafeArea(edges: .top)

            // content
            VStack {
                HStack {
                    Text("Balance")
                        .font(.callout)
                        .foregroundStyle(.white.opacity(0.82))

                    Spacer()
                }

                HStack {
                    Text(balanceString)
                        .font(.title2)
                        .fontWeight(.bold)
                        .foregroundStyle(.white)

                    HStack(spacing: 0) {
                        Text(model.unit)
                            .font(.subheadline)
                            .padding(.trailing, 0)
                    }
                    .foregroundStyle(.white)
                    .onTapGesture {
                        showingMenu.toggle()
                    }
                    .popover(isPresented: $showingMenu) {
                        VStack(alignment: .center, spacing: 8) {
                            Button("sats") {
                                model.dispatch(action: .updateUnit(.sat))
                                showingMenu = false
                            }
                            .buttonStyle(.plain)

                            Divider()

                            Button("btc") {
                                model.dispatch(action: .updateUnit(.btc))
                                showingMenu = false
                            }
                            .buttonStyle(.plain)
                        }
                        .padding(.vertical, 8)
                        .padding(.horizontal, 12)
                        .frame(minWidth: 120, maxWidth: 200)
                        .presentationCompactAdaptation(.popover)
                        .foregroundStyle(.primary.opacity(0.8))
                    }

                    Spacer()

                    Button(action: { model.dispatch(action: .toggleSensitiveVisibility) }) {
                        switch metadata.sensitiveVisible {
                        case true: Image(systemName: "eye.slash")
                        case false: Image(systemName: "eye")
                        }
                    }
                    .foregroundStyle(.white)
                }
            }
            // </content>
            .padding()
        }
        .frame(height: screenHeight * 0.20)
        .background(Color.midnightBlue)
    }
}

#Preview {
    struct Container: View {
        @State var model: WalletViewModel = .init(preview: "preview_only")

        var body: some View {
            SendFlowHeaderView(model: model, amount: Amount.fromSat(sats: 1_385_433))
        }
    }

    return AsyncPreview { Container() }
}
