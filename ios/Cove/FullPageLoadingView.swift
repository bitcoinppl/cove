//
//  FullPageLoadingView.swift
//  Cove
//
//  Created by Praveen Perera on 01/28/25.
//

import SwiftUI

struct FullPageLoadingView: View {
    var body: some View {
        ZStack {
            Color.coveBg.edgesIgnoringSafeArea(.all)
            ProgressView()
                .frame(width: 100, height: 100)
                .tint(.primary)
        }
    }
}

#Preview {
    FullPageLoadingView()
}
