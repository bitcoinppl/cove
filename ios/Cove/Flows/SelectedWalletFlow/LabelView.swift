//
//  LabelView.swift
//  Cove
//
//  Created by Praveen Perera on 2/13/25.
//

import SwiftUI

struct LabelView: View {
    @Environment(AppManager.self) private var app
    @Environment(WalletManager.self) private var manager

    var label: String? = nil

    var body: some View {
        Group {
            if let label {
                HStack {
                    Image(systemName: "tag.circle.fill")
                    Text(label)
                        .foregroundStyle(.secondary)
                }
            } else {
                HStack {
                    Image(systemName: "plus.circle.fill")
                        .symbolRenderingMode(.multicolor)

                    Text("Add label")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .font(.footnote)
    }
}

#Preview("No Label") {
    AsyncPreview {
        LabelView()
            .environment(AppManager.shared)
            .environment(WalletManager(preview: "preview_only"))
    }
}

#Preview("With Label") {
    AsyncPreview {
        LabelView(label: "Sent money for bike")
            .environment(AppManager.shared)
            .environment(WalletManager(preview: "preview_only"))
    }
}
