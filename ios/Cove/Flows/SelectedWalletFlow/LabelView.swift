//
//  LabelView.swift
//  Cove
//
//  Created by Praveen Perera on 2/13/25.
//

import SwiftUI

struct LabelView: View {
    @Environment(AppManager.self) private var app

    var label: String? = nil
    let manager: WalletManager

    var labelManager: LabelManager {
        manager.rust.labelManager()
    }

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
        LabelView(
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}

#Preview("With Label") {
    AsyncPreview {
        LabelView(
            label: "Sent money for bike",
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}
