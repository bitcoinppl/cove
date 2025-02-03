//
//  SettingsNetworkView.swift
//  Cove
//
//  Created by Praveen Perera on 1/30/25.
//

import SwiftUI

protocol SettingsEnum: CustomStringConvertible & CaseIterable & Hashable {
    var symbol: String { get }
}

extension SettingsEnum {
    var symbol: String { "" }
}

struct SettingsPicker<T: SettingsEnum>: View where T.AllCases: RandomAccessCollection {
    @Binding var selection: T

    var body: some View {
        Form {
            ForEach(T.allCases, id: \.self) { item in
                HStack {
                    if !item.symbol.isEmpty {
                        Image(systemName: item.symbol)
                    }

                    Text(item.description)
                        .font(.subheadline)

                    Spacer()

                    if selection == item {
                        Image(systemName: "checkmark")
                            .foregroundStyle(.blue)
                            .font(.footnote)
                            .fontWeight(.semibold)
                    }
                }
                .contentShape(Rectangle())
                .onTapGesture {
                    selection = item
                }
            }
        }
        .scrollContentBackground(.hidden)
    }
}

#Preview {
    SettingsContainer(route: .network)
        .environment(AppManager.shared)
}
