//
//  SettingsNetworkView.swift
//  Cove
//
//  Created by Praveen Perera on 1/30/25.
//

import SwiftUI

struct SettingsPicker<T: CaseIterable & Hashable & CustomStringConvertible>: View where T.AllCases: RandomAccessCollection {
    @Binding var selection: T

    var body: some View {
        Form {
            ForEach(T.allCases, id: \.self) { item in
                HStack {
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
        .onChange(of: selection) { o, n in
            print("Selection changed from \(o) to \(n)")
        }
    }
}

#Preview {
    SettingsContainer(route: .network)
        .environment(AppManager.shared)
}
