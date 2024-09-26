//
//  ScannerView.swift
//  Cove
//
//  Created by Praveen Perera on 9/25/24.
//

import AVFoundation
import Foundation
import SwiftUI

struct ScannerView: View {
    // init
    var codeTypes: [AVMetadataObject.ObjectType] = [.qr]
    var scanMode: ScanMode = .once
    var scanInterval: Double = 0.1
    var simulatedData: String = "Simulated Data"
    var showTorchButton: Bool = true
    var showFocusIndicator: Bool = true
    var focusIndicatorSize: CGFloat = 175
    var focusIndicatorColor: Color = .yellow
    @State var codeSize = 50.0
    var completion: (Result<ScanResult, ScanError>) -> Void = { _ in () }

    // private
    @State private var isTorchOn = false

    @State private var containerWidth: CGFloat = UIScreen.main.bounds.width
    @State private var containerHeight: CGFloat = UIScreen.main.bounds.height

    let startingCodeSize: CGFloat = 50
    let minimumCodeSize: CGFloat = 5
    let tapDownBy: CGFloat = 15

    var body: some View {
        GeometryReader { geo in
            ZStack {
                CodeScannerView(
                    codeTypes: codeTypes,
                    scanMode: scanMode,
                    scanInterval: scanInterval,
                    simulatedData: simulatedData,
                    isTorchOn: showTorchButton ? isTorchOn : false,
                    videoCaptureDevice: AVCaptureDevice.zoomedCameraForQRCode(withMinimumCodeSize: Float(codeSize)),
                    completion: completion
                )

                // Focus indicator
                if showFocusIndicator {
                    Image(systemName: "viewfinder")
                        .resizable()
                        .aspectRatio(contentMode: .fit)
                        .foregroundColor(focusIndicatorColor)
                        .frame(width: focusIndicatorSize, height: focusIndicatorSize)
                        .font(.system(size: focusIndicatorSize, weight: .ultraLight))
                        .position(
                            x: 0.5 * containerWidth,
                            y: 0.5 * containerHeight
                        )
                }

                if showTorchButton {
                    VStack {
                        Spacer()
                        Button(action: { isTorchOn.toggle() }) {
                            Image(systemName: isTorchOn ? "bolt.fill" : "bolt.slash.fill")
                                .foregroundColor(.white)
                                .padding()
                                .background(Color.black.opacity(0.7))
                                .clipShape(Circle())
                        }
                        .padding(.bottom, 40)
                    }
                }
            }
            .onAppear {
                containerWidth = geo.size.width
                containerHeight = geo.size.height
            }
        }
        .gesture(
            SpatialTapGesture()
                .onEnded { _ in
                    if codeSize <= minimumCodeSize {
                        withAnimation {
                            codeSize = startingCodeSize
                        }
                        return
                    }

                    withAnimation {
                        codeSize = max(minimumCodeSize, codeSize - tapDownBy)
                    }
                }
        )
        .gesture(
            TapGesture(count: 2)
                .onEnded {
                    codeSize = startingCodeSize
                }
        )
    }
}

#Preview {
    ScannerView()
}

#Preview("small") {
    VStack {
        Spacer()
        ScannerView()
            .padding()
            .background(.white)
            .frame(width: 300, height: 400)
        Spacer()
    }
}
