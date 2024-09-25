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
    var completion: (Result<ScanResult, ScanError>) -> Void = { _ in () }

    // private
    @State private var isTorchOn = false
    @State private var focusPoint = CGPoint(x: 0.5, y: 0.5)

    @State private var containerWidth: CGFloat = UIScreen.main.bounds.width
    @State private var containerHeight: CGFloat = UIScreen.main.bounds.height

    var body: some View {
        GeometryReader { geo in
            ZStack {
                CodeScannerView(
                    codeTypes: codeTypes,
                    scanMode: scanMode,
                    scanInterval: scanInterval,
                    simulatedData: simulatedData,
                    isTorchOn: showTorchButton ? isTorchOn : false,
                    completion: completion
                )

                // Focus indicator
                if showFocusIndicator {
                    Rectangle()
                        .stroke(focusIndicatorColor, lineWidth: 3)
                        .frame(width: focusIndicatorSize, height: focusIndicatorSize, alignment: .center)
                        .position(
                            x: focusPoint.x * containerWidth,
                            y: focusPoint.y * containerHeight
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
                .onEnded { value in
                    withAnimation {
                        focusPoint = CGPoint(
                            x: value.location.x / containerWidth,
                            y: value.location.y / containerHeight
                        )
                    }
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
