//
//  NewHotWalletView.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct NewHotWalletView: View {
    var route: HotWalletRoute

    var body: some View {
        switch route {
        case .select:
            VStack {
                NavigationLink(value: HotWalletRoute.create.intoRoute()) {
                    Text("Create Wallet")
                        .font(.title3)
                        .bold()
                        .foregroundStyle(.white)
                        .frame(minWidth: 250, minHeight: 100)
                }
                .background(
                    RoundedRectangle(cornerRadius: 15)
                        .fill(.green)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 15)
                        .stroke(Color.green, lineWidth: 2)
                        .brightness(-0.1)
                )
                .padding(.vertical, 15)

                NavigationLink(value: HotWalletRoute.import) {
                    Text("Restore Wallet")
                        .font(.title3)
                        .bold()
                        .foregroundStyle(.black)
                        .frame(minWidth: 250, minHeight: 100)
                }
                .overlay(
                    RoundedRectangle(cornerRadius: 15)
                        .stroke(Color.black, lineWidth: 2)
                )
                .padding(.vertical, 15)
            }
        case .create:
            Text("Create a new hot wallet...")
        case .import:
            Text("Import a new hot wallet...")
        }
    }
}

#Preview("Select") {
    NewHotWalletView(route: HotWalletRoute.select)
}

#Preview("Create") {
    NewHotWalletView(route: HotWalletRoute.create)
}

#Preview("Import") {
    NewHotWalletView(route: HotWalletRoute.import)
}
