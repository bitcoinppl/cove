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
    /// The white border around the code, in modules, for a code shown on a background a camera
    /// could mistake for part of it. Below CoreImage's own one-module border it has no effect
    var quietZoneModules = 1

    var body: some View {
        generateQRCode(text: text)
            .interpolation(.none)
            .resizable()
            .aspectRatio(1, contentMode: .fit)
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
            // CoreImage draws the code one point per module with a one-module white border
            let extraModules = CGFloat(max(0, quietZoneModules - 1))
            let extent = outputImage.extent.insetBy(dx: -extraModules, dy: -extraModules)
            let bordered = outputImage.composited(over: CIImage(color: .white).cropped(to: extent))

            if let cgImage = context.createCGImage(bordered, from: extent) {
                return Image(uiImage: UIImage(cgImage: cgImage))
            }
        }

        return Image(systemName: "exclamationmark.octagon")
    }
}

#Preview {
    QrCodeView(text: "hello")
}
