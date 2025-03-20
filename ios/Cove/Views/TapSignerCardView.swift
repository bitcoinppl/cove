//
//  TapSignerCardView.swift
//  Cove
//
//  Created by Praveen Perera on 3/20/25.
//

import SwiftUI

struct TapSignerCardView: View {
    var body: some View {
        VStack {
            Image(.tapSignerCard)
                .frame(width: 200, height: 200)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

#Preview {
    TapSignerCardView()
        .background(Color(hex: "3A4254"))
}
