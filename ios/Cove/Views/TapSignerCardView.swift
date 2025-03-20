//
//  TapSignerCardView.swift
//  Cove
//
//  Created by Praveen Perera on 3/20/25.
//

import SwiftUI

struct TapSignerCardView: View {
    static var backupPassword = "7A3F1E9B7C2D8405F6G0H9I4J2K7L5M6"
    static let cardId = "JGKLM-RTYUI-OPLMN-BVCXZ"
    static let width = 150.0

    var body: some View {
        VStack {
            ZStack(alignment: .top) {
                VStack(spacing: 20) {
                    VStack {
                        Text("Card identifier")

                        Text(Self.cardId)
                    }

                    RoundedRectangle(cornerRadius: 25) // Rounded rectangle for PIN
                        .stroke(Color.red, lineWidth: 2) // Red border
                        .frame(width: Self.width, height: 40) // Adjust size as needed
                        .overlay(
                            Text("Starting PIN code\n123 123")
                                .multilineTextAlignment(.center)
                        )

                    VStack {
                        Text("Backup Password")

                        Text(Self.backupPassword)
                    }
                }
                .padding()

                // Side text (rotated)
                VStack {
                    HStack(alignment: .top) {
                        Text(Self.backupPassword)
                            .tracking(5)
                            .rotationEffect(.degrees(90))
                            .frame(width: Self.width, height: 200)
                            .offset(x: -40)
                            .font(.system(size: 6))
                            .lineLimit(1)

                        Spacer()

                        Text(Self.backupPassword)
                            .tracking(5)
                            .rotationEffect(.degrees(90))
                            .frame(width: Self.width, height: 200)
                            .offset(x: 40)
                            .font(.system(size: 6))
                            .lineLimit(1)
                    }
                }
            }
        }
        .font(.system(size: 10))
        .background(.white)
        .frame(width: Self.width)
    }
}

#Preview {
    VStack {
        TapSignerCardView()
    }
    .frame(maxWidth: .infinity, maxHeight: .infinity)
    .background(Color(hex: "3A4254"))
}
