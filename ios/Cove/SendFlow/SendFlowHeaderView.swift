//
//  SendFlowHeaderView.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//

import Foundation
import SwiftUI

struct SendFlowHeaderView: View {
    @Environment(\.presentationMode) var presentationMode

    @Bindable var model: WalletViewModel
    let amount: Amount

    // private
    @State var showingMenu: Bool = false
    private let topPadding: CGFloat = 50
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

    private var unitString: String {
        return switch metadata.selectedUnit {
        case .btc: "btc"
        case .sat: "sats"
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            // Custom navigation bar
            HStack {
                Button(action: {
                    presentationMode.wrappedValue.dismiss()
                }) {
                    HStack(spacing: 0) {
                        Image(systemName: "chevron.left")
                            .fontWeight(.semibold)
                            .padding(.horizontal, 0)
                        Text("Back")
                            .offset(x: 5)
                    }
                    .offset(x: -8)
                    .foregroundStyle(.white)
                }

                Spacer()

                Text("Send")
                    .foregroundColor(.white)
                    .fontWeight(.medium)
                    .padding(.trailing, 50)

                Spacer()
            }
            .padding()
            .padding(.top, topPadding)

            Spacer()
            Spacer()
            Spacer()
            Spacer()

            VStack {
                HStack {
                    Text("Balance")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    Spacer()
                }

                HStack {
                    Text(balanceString)
                        .font(.title2)
                        .fontWeight(.bold)

                    HStack(spacing: 0) {
                        Text(unitString)
                            .font(.subheadline)
                            .padding(.trailing, 0)

                        Image(systemName: "chevron.down")
                            .font(.caption)
                            .fontWeight(.bold)
                            .padding(.top, 2)
                            .padding(.leading, 4)
                    }
                    .onTapGesture {
                        showingMenu.toggle()
                    }
                    .popover(isPresented: $showingMenu) {
                        VStack {
                            Button("sats") {
                                model.dispatch(action: .updateUnit(.sat))
                                showingMenu = false
                            }
                            Button("btc") {
                                model.dispatch(action: .updateUnit(.btc))
                                showingMenu = false
                            }
                        }
                        .padding()
                    }

                    Spacer()

                    Button(action: { model.dispatch(action: .toggleSensitiveVisibility) }) {
                        switch metadata.sensitiveVisible {
                        case true: Image(systemName: "eye.slash")
                        case false: Image(systemName: "eye")
                        }
                    }
                }
                .padding(.top, 2)

                Spacer()
            }
            .padding()
        }
        .background(
            Image(.headerPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(
                    width: 400, height: 300,
                    alignment: .topTrailing
                )
                .clipped()
                .ignoresSafeArea(.all)
        )
        .foregroundStyle(.white)
        .ignoresSafeArea(.all)
        .frame(width: screenWidth, height: screenHeight * 0.30)
        .background(Color.midnightBlue)
        .toolbarColorScheme(.dark, for: .navigationBar)
        .toolbar(.hidden, for: .navigationBar)
        .navigationBarBackButtonHidden()
        .edgesIgnoringSafeArea(.top)
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
