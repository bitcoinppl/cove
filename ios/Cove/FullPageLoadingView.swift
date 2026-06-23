//
//  FullPageLoadingView.swift
//  Cove
//
//  Created by Praveen Perera on 01/28/25.
//

import SwiftUI

struct FullPageLoadingView: View {
    var title: String?
    var backgroundColor: Color?
    var spinnerTint: Color
    var controlSize: ControlSize
    var ignoresSafeArea: Bool

    init(
        title: String? = nil,
        backgroundColor: Color? = .white,
        spinnerTint: Color = .primary,
        controlSize: ControlSize = .extraLarge,
        ignoresSafeArea: Bool = true
    ) {
        self.title = title
        self.backgroundColor = backgroundColor
        self.spinnerTint = spinnerTint
        self.controlSize = controlSize
        self.ignoresSafeArea = ignoresSafeArea
    }

    var body: some View {
        ZStack {
            if let backgroundColor {
                if ignoresSafeArea {
                    backgroundColor.ignoresSafeArea()
                } else {
                    backgroundColor
                }
            }

            VStack(spacing: 16) {
                ProgressView()
                    .progressViewStyle(.circular)
                    .controlSize(controlSize)
                    .frame(width: 80, height: 80)
                    .tint(spinnerTint)

                if let title {
                    Text(title)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
            }
            .accessibilityLabel(Text(title ?? "Loading"))
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

#Preview {
    FullPageLoadingView()
}
