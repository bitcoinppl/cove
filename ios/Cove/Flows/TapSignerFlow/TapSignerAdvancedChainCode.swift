//
//  TapSignerAdvancedChainCode.swift
//  Cove
//
//  Created by Praveen Perera on 3/24/25.
//

import SwiftUI

struct TapSignerAdvancedChainCode: View {
    @Environment(\.sizeCategory) private var sizeCategory
    @Environment(AppManager.self) var app
    @Environment(TapSignerManager.self) var manager

    let tapSigner: TapSigner

    /// private
    @State private var chainCode: String = ""

    private var isButtonDisabled: Bool {
        !isValidChainCode(chainCode: chainCode)
    }

    var body: some View {
        GeometryReader { proxy in
            let scrollableLayout = usesCompactLayout(
                sizeCategory: sizeCategory,
                availableHeight: proxy.size.height
            )

            Group {
                if scrollableLayout {
                    ScrollView {
                        mainContent(usesFlexibleSpacing: false)
                            .frame(minHeight: proxy.size.height, maxHeight: .infinity, alignment: .top)
                            .safeAreaPadding(.bottom, 24)
                    }
                    .scrollIndicators(.hidden)
                } else {
                    mainContent(usesFlexibleSpacing: true)
                }
            }
        }
        .contentTransition(.opacity)
        .background(TapSignerResultBackground())
        .navigationBarHidden(true)
    }

    private func mainContent(usesFlexibleSpacing: Bool) -> some View {
        VStack(spacing: 20) {
            HStack {
                Button(action: { manager.popRoute() }) {
                    Image(systemName: "chevron.left")
                    Text("Back")
                }

                Spacer()
            }
            .padding(.top, 20)
            .padding(.horizontal, 10)
            .foregroundStyle(.primary)
            .fontWeight(.semibold)

            if usesFlexibleSpacing {
                Spacer()
            }

            VStack {
                Text("Advanced Setup")
                    .font(.largeTitle)
                    .fontWeight(.bold)
                    .padding(.bottom, 5)
            }

            VStack(spacing: 12) {
                Group {
                    Text("Enter your custom 32-byte chain code below. If you’re unsure, select automatic on the previous screen.")
                }
                .font(.callout)
                .opacity(0.9)
                .multilineTextAlignment(.center)
            }
            .padding(.horizontal, 30)

            // Automatic Setup Button
            HStack {
                TextField("Enter a 32 byte hex string", text: $chainCode, axis: .vertical)
                    .lineLimit(4)
                    .font(.subheadline)
                    .frame(height: 100)

                Spacer()
            }
            .padding()
            .background(Color(.systemGray6))
            .cornerRadius(10)
            .padding(.horizontal, 20)
            .foregroundStyle(.primary)
            .padding(.top, 10)

            Button(action: { chainCode = generateRandomChainCode() }) {
                Text("Generate new string for me")
                    .font(.footnote)
                    .fontWeight(.semibold)
                    .padding(.bottom, 30)
            }
            .contentShape(Rectangle())
            .padding(.bottom, usesFlexibleSpacing ? screenHeight * 0.1 : 0)

            Button("Continue") {
                manager.navigate(to: .startingPin(tapSigner: tapSigner, chainCode: chainCode))
            }
            .buttonStyle(
                DarkButtonStyle(
                    backgroundColor: isButtonDisabled ? .systemGray4 : .midnightBtn,
                    foregroundColor: isButtonDisabled ? .systemGray6 : .white
                )
            )
            .padding()
            .padding(.bottom, 30)
            .disabled(isButtonDisabled)
        }
    }
}

#Preview {
    let t = tapSignerPreviewNew(preview: true)
    TapSignerContainer(route: .initAdvanced(t))
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
