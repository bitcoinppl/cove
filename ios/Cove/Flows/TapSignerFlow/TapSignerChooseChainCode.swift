//
//  TapSignerChooseChainCode.swift
//  Cove
//
//  Created by Praveen Perera on 3/19/25.
//

import SwiftUI

struct TapSignerChooseChainCode: View {
    var body: some View {
        VStack {
            // Top Cancel Button
            HStack {
                Button(action: {
                    // Action for Cancel button
                }) {
                    Text("Cancel")
                        .foregroundColor(.primary)
                        .fontWeight(.semibold)
                        .padding()
                }
                .padding(.top, 10)
                .padding(.horizontal, 5)
                Spacer()
            }
            .padding(.top, 10)

            Spacer()

            // Title with Underline
            VStack {
                Text("Setup Chain Code")
                    .font(.largeTitle)
                    .fontWeight(.bold)
                    .padding(.bottom, 5)
            }

            // Description Text
            VStack(spacing: 12) {
                Group {
                    Text("A chain code works with your private key to generate Bitcoin addresses")

                    Text("You can provide your own chain code for advanced setups, or let the app create one automatically for easy setup.")
                }
                .font(.callout)
                .opacity(0.9)
                .multilineTextAlignment(.center)
            }
            .padding(.horizontal, 30)
            .padding(.top, 20)

            // Automatic Setup Button
            Button(action: {
                // Action for Automatic Setup
            }) {
                VStack(spacing: 4) {
                    HStack {
                        Text("Automatic Setup")
                            .font(.footnote)
                            .fontWeight(.semibold)
                            .foregroundColor(.primary)

                        Spacer()

                        Image(systemName: "chevron.right")
                            .foregroundColor(.gray)
                    }

                    HStack {
                        Text("Let the app create a chain code for you")
                            .font(.footnote)
                            .foregroundStyle(.primary)

                        Spacer()
                    }
                }
                .padding()
                .background(Color(.systemGray6))
                .cornerRadius(10)
                .padding(.horizontal, 20)
            }
            .foregroundStyle(.primary)
            .padding(.top, 50)

            Spacer()

            // Advanced Setup Link
            Button(action: {
                // Action for Advanced Setup
            }) {
                Text("Advanced Setup")
                    .font(.footnote)
                    .fontWeight(.semibold)
                    .padding(.bottom, 30)
            }
            .contentShape(Rectangle())
        }
        .edgesIgnoringSafeArea(.all)
        .background(
            VStack {
                Image(.chainCodePattern)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .ignoresSafeArea(edges: .all)
                    .padding(.top, 5)

                Spacer()
            }
            .opacity(0.8)
        )
    }
}

#Preview {
    TapSignerChooseChainCode()
}
