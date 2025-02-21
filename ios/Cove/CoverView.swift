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
                .frame(width: 144, height: 144)
                .aspectRatio(contentMode: .fit)
                .cornerRadius(25.263)
        }
    }
}

#Preview {
    CoverView()
}
