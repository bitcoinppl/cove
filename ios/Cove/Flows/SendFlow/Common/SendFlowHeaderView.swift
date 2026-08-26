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

    private var metadata: WalletMetadata {
        manager.walletMetadata
    }

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
        SendFlowHeaderLayout(
            manager: manager,
            balanceString: balanceString,
            height: height
        )
    }
}

private struct SendFlowHeaderLayout: View {
    @Bindable var manager: WalletManager

    let balanceString: String
    let height: CGFloat

    @State private var showingMenu = false

    var body: some View {
        ZStack {
            Image(.headerPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(height: 225, alignment: .topTrailing)
                .frame(maxWidth: .infinity)
                .ignoresSafeArea(edges: .top)
                .clipped()

            SendFlowHeaderBalanceContent(
                manager: manager,
                balanceString: balanceString,
                showingMenu: $showingMenu
            )
            .padding()
        }
        .frame(height: height)
        .background(Color.midnightBlue)
    }
}

private struct SendFlowHeaderBalanceContent: View {
    @Bindable var manager: WalletManager

    let balanceString: String
    @Binding var showingMenu: Bool

    private var metadata: WalletMetadata {
        manager.walletMetadata
    }

    var body: some View {
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

                SendFlowHeaderUnitSelector(
                    manager: manager,
                    showingMenu: $showingMenu
                )

                Spacer()

                Button(action: toggleSensitiveVisibility) {
                    Image(systemName: metadata.sensitiveVisible ? "eye.slash" : "eye")
                }
                .foregroundStyle(.white)
            }
        }
    }

    private func toggleSensitiveVisibility() {
        manager.dispatch(action: .toggleSensitiveVisibility)
    }
}

private struct SendFlowHeaderUnitSelector: View {
    @Bindable var manager: WalletManager
    @Binding var showingMenu: Bool

    var body: some View {
        HStack(spacing: 0) {
            Text(manager.unit)
                .font(.subheadline)
                .padding(.trailing, 0)
        }
        .foregroundStyle(.white)
        .onTapGesture { showingMenu.toggle() }
        .popover(isPresented: $showingMenu) {
            SendFlowHeaderUnitMenu(
                selectSats: selectSats,
                selectBtc: selectBtc
            )
        }
    }

    private func selectSats() {
        manager.dispatch(action: .updateUnit(.sat))
        showingMenu = false
    }

    private func selectBtc() {
        manager.dispatch(action: .updateUnit(.btc))
        showingMenu = false
    }
}

private struct SendFlowHeaderUnitMenu: View {
    let selectSats: () -> Void
    let selectBtc: () -> Void

    var body: some View {
        VStack(alignment: .center, spacing: 0) {
            Button("sats", action: selectSats)
                .padding(8)
                .buttonStyle(.plain)

            Divider()

            Button("btc", action: selectBtc)
                .padding(8)
                .buttonStyle(.plain)
        }
        .padding(.vertical, 8)
        .padding(.horizontal, 12)
        .frame(minWidth: 120, maxWidth: 200)
        .presentationCompactAdaptation(.popover)
        .foregroundStyle(.primary.opacity(0.8))
    }
}

#Preview {
    struct Container: View {
        @State var manager: WalletManager = .init(preview: .only)

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
        @State var manager: WalletManager = .init(preview: .only)

        var body: some View {
            SendFlowHeaderView(
                manager: manager, amount: Amount.fromSat(sats: 1_385_433),
                height: 55
            )
        }
    }

    return AsyncPreview { Container() }
}
