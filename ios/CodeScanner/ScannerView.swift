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
            ScannerStage(
                codeTypes: codeTypes,
                scanMode: scanMode,
                scanInterval: scanInterval,
                simulatedData: simulatedData,
                showTorchButton: showTorchButton,
                showFocusIndicator: showFocusIndicator,
                focusIndicatorSize: focusIndicatorSize,
                focusIndicatorColor: focusIndicatorColor,
                codeSize: codeSize,
                zoomLevel: zoomLevel,
                containerWidth: containerWidth,
                containerHeight: containerHeight,
                isActive: viewLoaded && !showingPermissionAlert && scanError == nil,
                isTorchOn: $isTorchOn,
                toggleZoom: toggleZoom,
                completion: completeScan
            )
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

private struct ScannerStage: View {
    let codeTypes: [AVMetadataObject.ObjectType]
    let scanMode: ScanMode
    let scanInterval: Double
    let simulatedData: String
    let showTorchButton: Bool
    let showFocusIndicator: Bool
    let focusIndicatorSize: CGFloat
    let focusIndicatorColor: Color
    let codeSize: CGFloat
    let zoomLevel: String
    let containerWidth: CGFloat
    let containerHeight: CGFloat
    let isActive: Bool
    @Binding var isTorchOn: Bool
    let toggleZoom: () -> Void
    let completion: (Result<ScanResult, ScanError>) -> Void

    var body: some View {
        ZStack {
            ScannerCamera(
                codeTypes: codeTypes,
                scanMode: scanMode,
                scanInterval: scanInterval,
                simulatedData: simulatedData,
                codeSize: codeSize,
                isActive: isActive,
                isTorchOn: showTorchButton && isTorchOn,
                completion: completion
            )

            ScannerFocusIndicator(
                isVisible: showFocusIndicator,
                size: focusIndicatorSize,
                color: focusIndicatorColor,
                containerWidth: containerWidth,
                containerHeight: containerHeight
            )

            ScannerControls(
                showTorchButton: showTorchButton,
                isTorchOn: $isTorchOn,
                zoomLevel: zoomLevel,
                toggleZoom: toggleZoom
            )
        }
    }
}

private struct ScannerCamera: View {
    let codeTypes: [AVMetadataObject.ObjectType]
    let scanMode: ScanMode
    let scanInterval: Double
    let simulatedData: String
    let codeSize: CGFloat
    let isActive: Bool
    let isTorchOn: Bool
    let completion: (Result<ScanResult, ScanError>) -> Void

    var body: some View {
        if isActive {
            CodeScannerView(
                codeTypes: codeTypes,
                scanMode: scanMode,
                scanInterval: scanInterval,
                simulatedData: simulatedData,
                isTorchOn: isTorchOn,
                videoCaptureDevice: AVCaptureDevice.zoomedCameraForQRCode(
                    withMinimumCodeSize: Float(codeSize)
                ),
                completion: completion
            )
        }
    }
}

private struct ScannerFocusIndicator: View {
    let isVisible: Bool
    let size: CGFloat
    let color: Color
    let containerWidth: CGFloat
    let containerHeight: CGFloat

    var body: some View {
        if isVisible {
            Image(systemName: "viewfinder")
                .resizable()
                .aspectRatio(contentMode: .fit)
                .foregroundColor(color)
                .frame(width: size, height: size)
                .font(.system(size: size, weight: .ultraLight))
                .position(
                    x: 0.5 * containerWidth,
                    y: 0.5 * containerHeight
                )
        }
    }
}

private struct ScannerControls: View {
    let showTorchButton: Bool
    @Binding var isTorchOn: Bool
    let zoomLevel: String
    let toggleZoom: () -> Void

    var body: some View {
        HStack(spacing: 25) {
            if showTorchButton {
                ScannerControlButton(
                    systemImage: isTorchOn ? "bolt.fill" : "bolt.slash.fill",
                    action: { isTorchOn.toggle() }
                )
            }

            ScannerControlButton(title: zoomLevel) {
                withAnimation {
                    toggleZoom()
                }
            }
        }
    }
}

private struct ScannerControlButton: View {
    private enum Label {
        case title(String)
        case systemImage(String)
    }

    private let label: Label
    let action: () -> Void

    init(title: String, action: @escaping () -> Void) {
        label = .title(title)
        self.action = action
    }

    init(systemImage: String, action: @escaping () -> Void) {
        label = .systemImage(systemImage)
        self.action = action
    }

    var body: some View {
        VStack {
            Spacer()

            Button(action: action) {
                Group {
                    switch label {
                    case let .title(title):
                        Text(title)

                    case let .systemImage(systemImage):
                        Image(systemName: systemImage)
                    }
                }
                .foregroundColor(.white)
                .padding()
                .background(Color.black.opacity(0.7))
                .clipShape(Circle())
            }
            .padding(.bottom, 40)
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
