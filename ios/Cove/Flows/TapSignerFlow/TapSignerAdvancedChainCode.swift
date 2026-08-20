//
//  TapSignerAdvancedChainCode.swift
//  Cove
//
//  Created by Praveen Perera on 3/24/25.
//

import CoveCore
import SwiftUI

struct TapSignerAdvancedChainCode: View {
    @Environment(TapSignerManager.self) var manager

    let tapSigner: TapSigner

    /// private
    @State private var chainCode: String = ""

    private var isButtonDisabled: Bool {
        tapSignerChainCodeBytes(hex: chainCode) == nil
    }

    private var chainCodeError: String? {
        guard !chainCode.isEmpty else { return nil }
        return tapSignerChainCodeInputError(hex: chainCode)
    }

    var body: some View {
        TapSignerAdaptiveLayout { usesFlexibleSpacing in
            TapSignerAdvancedChainCodeContent(
                chainCode: $chainCode,
                isButtonDisabled: isButtonDisabled,
                chainCodeError: chainCodeError,
                usesFlexibleSpacing: usesFlexibleSpacing,
                backAction: goBack,
                generateAction: generateChainCode,
                continueAction: continueSetup
            )
        }
        .contentTransition(.opacity)
        .background(TapSignerResultBackground())
        .navigationBarHidden(true)
    }

    private func goBack() {
        manager.popRoute()
    }

    private func generateChainCode() {
        chainCode = generateRandomChainCode()
    }

    private func continueSetup() {
        guard tapSignerChainCodeBytes(hex: chainCode) != nil else { return }
        manager.navigate(to: .startingPin(tapSigner: tapSigner, chainCode: chainCode))
    }
}

private struct TapSignerAdvancedChainCodeContent: View {
    @Binding var chainCode: String

    let isButtonDisabled: Bool
    let chainCodeError: String?
    let usesFlexibleSpacing: Bool
    let backAction: () -> Void
    let generateAction: () -> Void
    let continueAction: () -> Void

    var body: some View {
        VStack(spacing: 20) {
            TapSignerTopActionHeader(
                "Back",
                systemImage: "chevron.left",
                action: backAction
            )

            TapSignerFlexibleSpacer(enabled: usesFlexibleSpacing)

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

            if let chainCodeError {
                Text(chainCodeError)
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 20)
            }

            Button(action: generateAction) {
                Text("Generate new string for me")
                    .font(.footnote)
                    .fontWeight(.semibold)
                    .padding(.bottom, 30)
            }
            .contentShape(Rectangle())
            .padding(.bottom, usesFlexibleSpacing ? screenHeight * 0.1 : 0)

            Button("Continue", action: continueAction)
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
