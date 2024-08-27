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

    private let pasteboard = UIPasteboard.general

    @State private var addressInfo: AddressInfo?

    var addressLoaded: Bool {
        addressInfo != nil
    }

    var accentColor: Color {
        model.accentColor
    }

    func copyText() {
        dismiss()

        if let addressInfo = addressInfo {
            pasteboard.string = addressInfo.adressString()
            FloaterPopup(text: "Address Copied")
                .showAndStack()
                .dismissAfter(2)
        }
    }

    func nextAddress() async {
        do {
            let addressInfo = try await model.rust.nextAddress()
            await MainActor.run {
                self.addressInfo = addressInfo
            }
        } catch {
            Log.error("Unable to get next address: \(error)")
            // TODO: error getting address handle?
        }
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

            AddressView(addressInfo: addressInfo)
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

                Button(action: {
                    Task {
                        await nextAddress()
                    }
                }) {
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
        .task {
            await nextAddress()
        }
    }
}

private struct AddressView: View {
    let addressInfo: AddressInfo?

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
        Group {
            if let addressInfo = addressInfo {
                GroupBox {
                    VStack {
                        Image(uiImage: generateQRCode(from: addressInfo.adressString()))
                            .interpolation(.none)
                            .resizable()
                            .scaledToFit()
                            .frame(width: 250, height: 250)
                            .padding()

                        Text(addressInfo.adressString())
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
            } else {
                ProgressView(label: {
                    Text("Loading")
                        .font(.caption)
                        .foregroundColor(.secondary)
                })
                .progressViewStyle(.circular)
            }
        }
    }
}

#Preview {
    AsyncPreview {
        ReceiveView(model: WalletViewModel(preview: "preview_only"))
            .environment(MainViewModel())
    }
}