//
//  ReceiveView.swift
//  Cove
//
//  Created by Praveen Perera on 8/14/24.
//

import CoreImage.CIFilterBuiltins
import SwiftUI

struct ReceiveView: View {
    @Environment(\.dismiss) private var dismiss

    let model: WalletViewModel
    @Binding var showingCopiedPopup: Bool

    private let pasteboard = UIPasteboard.general
    private let address = "bc1qmtxtcueces2runrampaakp8vtpmn7q9lmpuwgr"

    var accentColor: Color {
        model.accentColor
    }

    func copyText() {
        dismiss()
        pasteboard.string = address
        showingCopiedPopup = true
    }

    var body: some View {
        VStack {
            HStack {
                Text("Address")
                    .font(.headline)
                    .fontWeight(.bold)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal)
                    .padding(.top)
                Spacer()
            }

            AddressView(address: address)
                .padding(.bottom, 50)

            VStack {
                Button(action: copyText) {
                    HStack(spacing: 10) {
                        Image(systemName: "doc.on.doc")
                        Text("Copy Address")
                    }
                    .foregroundColor(.white)
                    .frame(maxWidth: .infinity)
                    .padding()
                    .background(accentColor)
                    .cornerRadius(8)
                }

                Button(action: {}) {
                    HStack(spacing: 10) {
                        Image(systemName: "arrow.triangle.2.circlepath")
                        Text("New Address")
                    }
                    .foregroundColor(accentColor)
                    .frame(maxWidth: .infinity)
                    .padding()
                    .background(Color.white)
                    .cornerRadius(8)
                    .overlay(
                        RoundedRectangle(cornerRadius: 8)
                            .stroke(accentColor, lineWidth: 1)
                    )
                }
            }
            .padding(.horizontal)
        }
    }
}

private struct AddressView: View {
    let address: String

    func generateQRCode(from string: String) -> UIImage {
        let context = CIContext()
        let filter = CIFilter.qrCodeGenerator()

        filter.message = Data(string.utf8)
        filter.correctionLevel = "M"

        if let outputImage = filter.outputImage {
            if let cgImage = context.createCGImage(outputImage, from: outputImage.extent) {
                return UIImage(cgImage: cgImage)
            }
        }

        return UIImage(systemName: "xmark.circle") ?? UIImage()
    }

    var body: some View {
        GroupBox {
            VStack {
                Image(uiImage: generateQRCode(from: address))
                    .interpolation(.none)
                    .resizable()
                    .scaledToFit()
                    .frame(width: 250, height: 250)
                    .padding()

                Text(address as String)
                    .font(.custom("Menlo", size: 18))
                    .multilineTextAlignment(.leading)
                    .minimumScaleFactor(0.01)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                    .padding(.top, 10)
                    .padding([.bottom, .horizontal])
            }
        }
        .padding()
    }
}

#Preview {
    AsyncPreview {
        ReceiveView(model: WalletViewModel(preview: "preview_only"), showingCopiedPopup: Binding.constant(false))
            .environment(MainViewModel())
    }
}
