//
//  AsyncPreview.swift
//  Cove
//
//  Created by Praveen Perera on 7/31/24.
//

import SwiftUI

struct AsyncPreview<Content: View>: View {
    let content: () async -> Content
    @State private var contentView: Content?

    init(@ViewBuilder content: @escaping () async -> Content) {
        self.content = content
    }

    @State private var manager = AppManager.shared

    var body: some View {
        Group {
            if let content = contentView {
                content
            } else {
                Text("Loading preview")
            }
        }
        .task {
            await manager.rust.initOnStart()
            contentView = await content()
        }
    }
}
