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

    @Bindable var manager: WalletManager
    let amount: Amount

    @State var height: CGFloat = screenHeight * 0.145

    // private
    @State var showingMenu: Bool = false
    private var metadata: WalletMetadata { manager.walletMetadata }
    private var balanceString: String {
        if !metadata.sensitiveVisible {
            return "••••••"
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
                .frame(height: 225, alignment: .topTrailing)
                .frame(maxWidth: .infinity)
                .ignoresSafeArea(edges: .top)

            // content
            VStack {
                HStack {
                    Text("Balance")
                        .font(.footnote)
                        .foregroundStyle(.white.opacity(0.82))

                    Spacer()
                }
                .padding(.top, 35)

                HStack {
                    Text(balanceString)
                        .font(.title3)
                        .fontWeight(.bold)
                        .foregroundStyle(.white)

                    HStack(spacing: 0) {
                        Text(manager.unit)
                            .font(.subheadline)
                            .padding(.trailing, 0)
                    }
                    .foregroundStyle(.white)
                    .onTapGesture {
                        showingMenu.toggle()
                    }
                    .popover(isPresented: $showingMenu) {
                        VStack(alignment: .center, spacing: 0) {
                            Button("sats") {
                                manager.dispatch(action: .updateUnit(.sat))
                                showingMenu = false
                            }
                            .padding(8)
                            .buttonStyle(.plain)

                            Divider()

                            Button("btc") {
                                manager.dispatch(action: .updateUnit(.btc))
                                showingMenu = false
                            }
                            .padding(8)
                            .buttonStyle(.plain)
                        }
                        .padding(.vertical, 8)
                        .padding(.horizontal, 12)
                        .frame(minWidth: 120, maxWidth: 200)
                        .presentationCompactAdaptation(.popover)
                        .foregroundStyle(.primary.opacity(0.8))
                    }

                    Spacer()

                    Button(action: {
                        manager.dispatch(action: .toggleSensitiveVisibility)
                    }) {
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
        .frame(height: height)
        .background(Color.midnightBlue)
    }
}

#Preview {
    struct Container: View {
        @State var manager: WalletManager = .init(preview: "preview_only")

        var body: some View {
            SendFlowHeaderView(
                manager: manager, amount: Amount.fromSat(sats: 1_385_433)
            )
        }
    }

    return AsyncPreview { Container() }
}

#Preview("small") {
    struct Container: View {
        @State var manager: WalletManager = .init(preview: "preview_only")

        var body: some View {
            SendFlowHeaderView(
                manager: manager, amount: Amount.fromSat(sats: 1_385_433),
                height: 55
            )
        }
    }

    return AsyncPreview { Container() }
}
