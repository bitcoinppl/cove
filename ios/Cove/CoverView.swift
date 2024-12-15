//
//  CoverView.swift
//  Cove
//
//  Created by Praveen Perera on 12/15/24.
//

import SwiftUI

struct CoverView: View {
    var body: some View {
        ZStack {
            Color.black.edgesIgnoringSafeArea(.all)
            Image(.icon)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(width: 100, height: 100)
        }
    }
}

#Preview {
    CoverView()
}
