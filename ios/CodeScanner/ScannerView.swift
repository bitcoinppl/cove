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
    var focusIndicatorColor: Color = .white
    @State var codeSize = 40.0
    var showAlert = true
    var completion: (Result<ScanResult, ScanError>) -> Void = { _ in () }

    // private
    @State private var isTorchOn = false
    @State private var containerWidth: CGFloat = UIScreen.main.bounds.width
    @State private var containerHeight: CGFloat = UIScreen.main.bounds.height
    @State private var showingPermissionAlert: Bool = false
    @State private var scanError: ScanError?

    @State private var viewLoaded: Bool = false

    let startingCodeSize: CGFloat = 40
    let minimumCodeSize: CGFloat = 15
    let tapDownBy: CGFloat = 25

    var zoomLevel: String {
        switch codeSize {
        case 40.0: "1x"
        default: "2x"
        }
    }

    func toggleZoom() {
        if codeSize == startingCodeSize {
            codeSize = codeSize - tapDownBy
        } else {
            codeSize = startingCodeSize
        }
    }

    func completeScan(_ result: Result<ScanResult, ScanError>) {
        if !showAlert {
            return completion(result)
        }

        if case .failure(ScanError.permissionDenied) = result {
            DispatchQueue.main.async {
                showingPermissionAlert = true
                scanError = ScanError.permissionDenied
            }

            return
        }

        completion(result)
    }

    var body: some View {
        GeometryReader { geo in
            ZStack {
                if viewLoaded, !showingPermissionAlert, scanError == nil {
                    CodeScannerView(
                        codeTypes: codeTypes,
                        scanMode: scanMode,
                        scanInterval: scanInterval,
                        simulatedData: simulatedData,
                        isTorchOn: showTorchButton ? isTorchOn : false,
                        videoCaptureDevice: AVCaptureDevice.zoomedCameraForQRCode(withMinimumCodeSize: Float(codeSize)),
                        completion: completeScan
                    )
                }

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

                HStack(spacing: 25) {
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

                    VStack {
                        Spacer()
                        Button(action: {
                            withAnimation {
                                toggleZoom()
                            }
                        }) {
                            Text(zoomLevel)
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
                viewLoaded = true
            }
            .onTapGesture(perform: toggleZoom)
            .alert(isPresented: $showingPermissionAlert) {
                Alert(
                    title: Text("Camera Access Required"),
                    message: Text("Please allow camera access in Settings to use this feature."),
                    primaryButton: Alert.Button.default(Text("Settings")) {
                        let url = URL(string: UIApplication.openSettingsURLString)!
                        UIApplication.shared.open(url)
                    },
                    secondaryButton: Alert.Button.cancel {
                        Task {
                            await MainActor.run {
                                showingPermissionAlert = false
                                if let error = scanError {
                                    completion(.failure(error))
                                }
                            }
                        }
                    }
                )
            }
        }
    }
}

#Preview {
    VStack {
        ScannerView()
    }
    .background(.black)
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
