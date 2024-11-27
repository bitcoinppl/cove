//
//  QrCodeView.swift
//  Cove
//
//  Created by Praveen Perera on 11/24/24.
//

import CoreImage
import SwiftUI

struct QrCodeView: View {
    let text: String

    var body: some View {
        generateQRCode(text: text)
            .interpolation(.none)
            .resizable()
            .scaledToFit()
    }

    func generateQRCode(text: String) -> Image {
        let context = CIContext()
        let filter = CIFilter.qrCodeGenerator()

        guard let data = text.data(using: .ascii, allowLossyConversion: false) else {
            return Image(systemName: "exclamationmark.octagon")
        }

        filter.setValue(data, forKey: "inputMessage")
        filter.setValue("L", forKey: "inputCorrectionLevel")

        if let outputImage = filter.outputImage {
            if let cgImage = context.createCGImage(outputImage, from: outputImage.extent) {
                return Image(uiImage: UIImage(cgImage: cgImage))
            }
        }

        return Image(systemName: "exclamationmark.octagon")
    }
}

#Preview {
    QrCodeView(text: "hello")
}
