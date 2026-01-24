//
//  ScannerViewController.swift
//  https://github.com/twostraws/CodeScanner
//
//  Created by Paul Hudson on 14/12/2021.
//  Copyright © 2021 Paul Hudson. All rights reserved.
//

#if os(iOS)
    import AVFoundation
    import UIKit

    @available(macCatalyst 14.0, *)
    public extension CodeScannerView {
        final class ScannerViewController: UIViewController, UINavigationControllerDelegate {
            private let photoOutput = AVCapturePhotoOutput()
            private var isCapturing = false
            private var handler: ((UIImage?) -> Void)?
            var parentView: CodeScannerView!
            var codesFound = Set<ScanResultData>()
            var didFinishScanning = false
            var lastTime = Date(timeIntervalSince1970: 0)
            private let showViewfinder: Bool

            let fallbackVideoCaptureDevice = AVCaptureDevice.default(for: .video)

            private var isGalleryShowing: Bool = false {
                didSet {
                    // Update binding
                    if parentView.isGalleryPresented.wrappedValue != isGalleryShowing {
                        parentView.isGalleryPresented.wrappedValue = isGalleryShowing
                    }
                }
            }

            public init(showViewfinder: Bool = false, parentView: CodeScannerView) {
                self.parentView = parentView
                self.showViewfinder = showViewfinder
                super.init(nibName: nil, bundle: nil)
            }

            required init?(coder: NSCoder) {
                showViewfinder = false
                super.init(coder: coder)
            }

            func openGallery() {
                isGalleryShowing = true
                let imagePicker = UIImagePickerController()
                imagePicker.delegate = self
                imagePicker.presentationController?.delegate = self
                present(imagePicker, animated: true, completion: nil)
            }

            @objc func openGalleryFromButton(_: UIButton) {
                openGallery()
            }

            #if targetEnvironment(simulator)
                override public func loadView() {
                    view = UIView()
                    view.isUserInteractionEnabled = true

                    let label = UILabel()
                    label.translatesAutoresizingMaskIntoConstraints = false
                    label.numberOfLines = 0
                    label.text =
                        "You're running in the simulator, which means the camera isn't available. Tap anywhere to send back some simulated data."
                    label.textAlignment = .center

                    let button = UIButton()
                    button.translatesAutoresizingMaskIntoConstraints = false
                    button.setTitle("Select a custom image", for: .normal)
                    button.setTitleColor(UIColor.systemBlue, for: .normal)
                    button.setTitleColor(UIColor.gray, for: .highlighted)
                    button.addTarget(
                        self, action: #selector(openGalleryFromButton), for: .touchUpInside
                    )

                    let stackView = UIStackView()
                    stackView.translatesAutoresizingMaskIntoConstraints = false
                    stackView.axis = .vertical
                    stackView.spacing = 50
                    stackView.addArrangedSubview(label)
                    stackView.addArrangedSubview(button)

                    view.addSubview(stackView)

                    NSLayoutConstraint.activate([
                        button.heightAnchor.constraint(equalToConstant: 50),
                        stackView.leadingAnchor.constraint(
                            equalTo: view.layoutMarginsGuide.leadingAnchor),
                        stackView.trailingAnchor.constraint(
                            equalTo: view.layoutMarginsGuide.trailingAnchor),
                        stackView.centerYAnchor.constraint(equalTo: view.centerYAnchor),
                    ])
                }

                override public func touchesBegan(_: Set<UITouch>, with _: UIEvent?) {
                    // Send back their simulated data, as if it was one of the types they were scanning for
                    found(
                        ScanResult(
                            data: .string(parentView.simulatedData),
                            type: parentView.codeTypes.first ?? .qr, image: nil, corners: []
                        ))
                }

            #else

                var captureSession: AVCaptureSession?
                var previewLayer: AVCaptureVideoPreviewLayer!

                private lazy var viewFinder: UIImageView? = {
                    guard let image = UIImage(systemName: "viewfinder") else {
                        return nil
                    }

                    let imageView = UIImageView(image: image)
                    imageView.translatesAutoresizingMaskIntoConstraints = false
                    return imageView
                }()

                private lazy var manualCaptureButton: UIButton = {
                    let button = UIButton(type: .system)
                    let image = UIImage(systemName: "capture")
                    button.setBackgroundImage(image, for: .normal)
                    button.addTarget(
                        self, action: #selector(manualCapturePressed), for: .touchUpInside
                    )
                    button.translatesAutoresizingMaskIntoConstraints = false
                    return button
                }()

                private lazy var manualSelectButton: UIButton = {
                    let button = UIButton(type: .system)
                    let image = UIImage(systemName: "photo.on.rectangle")
                    let background = UIImage(systemName: "capsule.fill")?.withTintColor(
                        .systemBackground, renderingMode: .alwaysOriginal
                    )
                    button.setImage(image, for: .normal)
                    button.setBackgroundImage(background, for: .normal)
                    button.addTarget(
                        self, action: #selector(openGalleryFromButton), for: .touchUpInside
                    )
                    button.translatesAutoresizingMaskIntoConstraints = false
                    return button
                }()

                override public func viewDidLoad() {
                    super.viewDidLoad()
                    addOrientationDidChangeObserver()
                    setBackgroundColor()
                    handleCameraPermission()
                }

                override public func viewWillLayoutSubviews() {
                    previewLayer?.frame = view.layer.bounds
                }

                @objc func updateOrientation() {
                    guard let orientation = view.window?.windowScene?.interfaceOrientation else {
                        return
                    }
                    guard let connection = captureSession?.connections.last,
                          connection.isVideoOrientationSupported
                    else { return }
                    switch orientation {
                    case .portrait:
                        connection.videoOrientation = .portrait
                    case .landscapeLeft:
                        connection.videoOrientation = .landscapeLeft
                    case .landscapeRight:
                        connection.videoOrientation = .landscapeRight
                    case .portraitUpsideDown:
                        connection.videoOrientation = .portraitUpsideDown
                    default:
                        connection.videoOrientation = .portrait
                    }
                }

                override public func viewDidAppear(_ animated: Bool) {
                    super.viewDidAppear(animated)
                    updateOrientation()
                }

                override public func viewWillAppear(_ animated: Bool) {
                    super.viewWillAppear(animated)

                    setupSession()
                }

                private func setupSession() {
                    guard let captureSession else {
                        return
                    }

                    if previewLayer == nil {
                        previewLayer = AVCaptureVideoPreviewLayer(session: captureSession)
                    }

                    previewLayer.frame = view.layer.bounds
                    previewLayer.videoGravity = .resizeAspectFill
                    view.layer.addSublayer(previewLayer)
                    addViewFinder()

                    reset()

                    if !captureSession.isRunning {
                        DispatchQueue.global(qos: .userInteractive).async {
                            self.captureSession?.startRunning()
                        }
                    }
                }

                private func handleCameraPermission() {
                    switch AVCaptureDevice.authorizationStatus(for: .video) {
                    case .restricted:
                        break
                    case .denied:
                        didFail(reason: .permissionDenied)
                    case .notDetermined:
                        requestCameraAccess {
                            self.setupCaptureDevice()
                            DispatchQueue.main.async {
                                self.setupSession()
                            }
                        }
                    case .authorized:
                        setupCaptureDevice()
                        setupSession()
                    default:
                        break
                    }
                }

                private func requestCameraAccess(completion: (() -> Void)?) {
                    AVCaptureDevice.requestAccess(for: .video) { [weak self] status in
                        guard status else {
                            self?.didFail(reason: .permissionDenied)
                            return
                        }
                        completion?()
                    }
                }

                private func addOrientationDidChangeObserver() {
                    NotificationCenter.default.addObserver(
                        self,
                        selector: #selector(updateOrientation),
                        name: UIDevice.orientationDidChangeNotification,
                        object: nil
                    )
                }

                private func setBackgroundColor(_ color: UIColor = .black) {
                    view.backgroundColor = color
                }

                private func setupCaptureDevice() {
                    captureSession = AVCaptureSession()

                    guard
                        let videoCaptureDevice = parentView.videoCaptureDevice
                        ?? fallbackVideoCaptureDevice
                    else {
                        return
                    }

                    let videoInput: AVCaptureDeviceInput

                    do {
                        videoInput = try AVCaptureDeviceInput(device: videoCaptureDevice)
                    } catch {
                        didFail(reason: .initError(error))
                        return
                    }

                    if captureSession!.canAddInput(videoInput) {
                        captureSession!.addInput(videoInput)
                    } else {
                        didFail(reason: .badInput)
                        return
                    }
                    let metadataOutput = AVCaptureMetadataOutput()

                    if captureSession!.canAddOutput(metadataOutput) {
                        captureSession!.addOutput(metadataOutput)
                        captureSession!.addOutput(photoOutput)
                        metadataOutput.setMetadataObjectsDelegate(self, queue: DispatchQueue.main)
                        metadataOutput.metadataObjectTypes = parentView.codeTypes
                    } else {
                        didFail(reason: .badOutput)
                        return
                    }
                }

                private func addViewFinder() {
                    guard showViewfinder, let imageView = viewFinder else { return }

                    view.addSubview(imageView)

                    // 65% of smaller view dimension, capped between 200-320pt
                    let viewBounds = view.bounds
                    let smallerDimension = min(viewBounds.width, viewBounds.height)
                    let viewfinderSize = min(max(smallerDimension * 0.65, 200), 320)

                    NSLayoutConstraint.activate([
                        imageView.centerYAnchor.constraint(equalTo: view.centerYAnchor),
                        imageView.centerXAnchor.constraint(equalTo: view.centerXAnchor),
                        imageView.widthAnchor.constraint(equalToConstant: viewfinderSize),
                        imageView.heightAnchor.constraint(equalToConstant: viewfinderSize),
                    ])
                }

                override public func viewDidDisappear(_ animated: Bool) {
                    super.viewDidDisappear(animated)

                    if captureSession?.isRunning == true {
                        DispatchQueue.global(qos: .userInteractive).async {
                            self.captureSession?.stopRunning()
                        }
                    }

                    NotificationCenter.default.removeObserver(self)
                }

                override public var prefersStatusBarHidden: Bool {
                    true
                }

                override public var supportedInterfaceOrientations: UIInterfaceOrientationMask {
                    .all
                }

                /** Touch the screen for autofocus */
                override public func touchesBegan(_ touches: Set<UITouch>, with _: UIEvent?) {
                    guard touches.first?.view == view,
                          let touchPoint = touches.first,
                          let device = parentView.videoCaptureDevice ?? fallbackVideoCaptureDevice,
                          device.isFocusPointOfInterestSupported
                    else { return }

                    let videoView = view
                    let screenSize = videoView!.bounds.size
                    let xPoint = touchPoint.location(in: videoView).y / screenSize.height
                    let yPoint = 1.0 - touchPoint.location(in: videoView).x / screenSize.width
                    let focusPoint = CGPoint(x: xPoint, y: yPoint)

                    do {
                        try device.lockForConfiguration()
                    } catch {
                        return
                    }

                    // Focus to the correct point, make continuous focus and exposure so the point stays sharp when moving the device closer
                    device.focusPointOfInterest = focusPoint
                    device.focusMode = .continuousAutoFocus
                    device.exposurePointOfInterest = focusPoint
                    device.exposureMode = AVCaptureDevice.ExposureMode.continuousAutoExposure
                    device.unlockForConfiguration()
                }

                @objc func manualCapturePressed(_: Any?) {
                    readyManualCapture()
                }

                func showManualCaptureButton(_ isManualCapture: Bool) {
                    if manualCaptureButton.superview == nil {
                        view.addSubview(manualCaptureButton)
                        NSLayoutConstraint.activate([
                            manualCaptureButton.heightAnchor.constraint(equalToConstant: 60),
                            manualCaptureButton.widthAnchor.constraint(
                                equalTo: manualCaptureButton.heightAnchor),
                            view.centerXAnchor.constraint(
                                equalTo: manualCaptureButton.centerXAnchor),
                            view.safeAreaLayoutGuide.bottomAnchor.constraint(
                                equalTo: manualCaptureButton.bottomAnchor, constant: 32
                            ),
                        ])
                    }

                    view.bringSubviewToFront(manualCaptureButton)
                    manualCaptureButton.isHidden = !isManualCapture
                }

                func showManualSelectButton(_ isManualSelect: Bool) {
                    if manualSelectButton.superview == nil {
                        view.addSubview(manualSelectButton)
                        NSLayoutConstraint.activate([
                            manualSelectButton.heightAnchor.constraint(equalToConstant: 50),
                            manualSelectButton.widthAnchor.constraint(equalToConstant: 60),
                            view.centerXAnchor.constraint(
                                equalTo: manualSelectButton.centerXAnchor),
                            view.safeAreaLayoutGuide.bottomAnchor.constraint(
                                equalTo: manualSelectButton.bottomAnchor, constant: 32
                            ),
                        ])
                    }

                    view.bringSubviewToFront(manualSelectButton)
                    manualSelectButton.isHidden = !isManualSelect
                }
            #endif

            func updateViewController(
                isTorchOn: Bool, isGalleryPresented: Bool, isManualCapture: Bool,
                isManualSelect: Bool
            ) {
                guard
                    let videoCaptureDevice = parentView.videoCaptureDevice
                    ?? fallbackVideoCaptureDevice
                else {
                    return
                }

                if videoCaptureDevice.hasTorch {
                    try? videoCaptureDevice.lockForConfiguration()
                    videoCaptureDevice.torchMode = isTorchOn ? .on : .off
                    videoCaptureDevice.unlockForConfiguration()
                }

                if isGalleryPresented, !isGalleryShowing {
                    openGallery()
                }

                #if !targetEnvironment(simulator)
                    showManualCaptureButton(isManualCapture)
                    showManualSelectButton(isManualSelect)
                #endif
            }

            public func reset() {
                codesFound.removeAll()
                didFinishScanning = false
                lastTime = Date(timeIntervalSince1970: 0)
            }

            public func readyManualCapture() {
                guard parentView.scanMode.isManual else { return }
                reset()
                lastTime = Date()
            }

            var isPastScanInterval: Bool {
                Date().timeIntervalSince(lastTime) >= parentView.scanInterval
            }

            var isWithinManualCaptureInterval: Bool {
                Date().timeIntervalSince(lastTime) <= 0.5
            }

            func found(_ result: ScanResult) {
                lastTime = Date()
                parentView.completion(.success(result))
            }

            func didFail(reason: ScanError) {
                parentView.completion(.failure(reason))
            }
        }
    }

    // MARK: - AVCaptureMetadataOutputObjectsDelegate

    @available(macCatalyst 14.0, *)
    extension CodeScannerView.ScannerViewController: AVCaptureMetadataOutputObjectsDelegate {
        public func metadataOutput(
            _: AVCaptureMetadataOutput, didOutput metadataObjects: [AVMetadataObject],
            from _: AVCaptureConnection
        ) {
            guard let metadataObject = metadataObjects.first,
                  !parentView.isPaused,
                  !didFinishScanning,
                  !isCapturing,
                  let readableObject = metadataObject as? AVMetadataMachineReadableCodeObject
            else {
                return
            }

            let data: ScanResultData? = {
                switch (readableObject.stringValue, readableObject.descriptor) {
                case (nil, nil): return .none
                case let (.some(stringValue), _): return .string(stringValue)
                case let (_, .some(descriptor)):
                    if let descriptor = descriptor as? CIQRCodeDescriptor {
                        let decoder = BinaryDecoder()

                        let payload = descriptor.errorCorrectedPayload
                        let symbolVersion = descriptor.symbolVersion

                        let data = decoder.decodeQRErrorCorrectedBytes(
                            payload, symbolVersion: symbolVersion
                        )!
                        return ScanResultData.data(data)
                    } else {
                        return .none
                    }
                }
            }()

            guard let data else { return }

            handler = { [weak self] image in
                guard let self else { return }
                let result = ScanResult(
                    data: data, type: readableObject.type, image: image,
                    corners: readableObject.corners
                )

                switch parentView.scanMode {
                case .once:
                    found(result)
                    // make sure we only trigger scan once per use
                    didFinishScanning = true

                case .manual:
                    if !didFinishScanning, isWithinManualCaptureInterval {
                        found(result)
                        didFinishScanning = true
                    }

                case .oncePerCode:
                    if !codesFound.contains(data) {
                        codesFound.insert(data)
                        found(result)
                    }

                case .continuous:
                    if isPastScanInterval {
                        found(result)
                    }

                case let .continuousExcept(ignoredList):
                    if isPastScanInterval, !ignoredList.contains(data) {
                        found(result)
                    }
                }
            }

            if parentView.requiresPhotoOutput {
                isCapturing = true
                photoOutput.capturePhoto(with: AVCapturePhotoSettings(), delegate: self)
            } else {
                handler?(nil)
            }
        }
    }

    // MARK: - UIImagePickerControllerDelegate

    @available(macCatalyst 14.0, *)
    extension CodeScannerView.ScannerViewController: UIImagePickerControllerDelegate {
        public func imagePickerController(
            _: UIImagePickerController,
            didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]
        ) {
            isGalleryShowing = false

            defer {
                dismiss(animated: true)
            }

            guard let qrcodeImg = info[.originalImage] as? UIImage,
                  let detector = CIDetector(
                      ofType: CIDetectorTypeQRCode, context: nil,
                      options: [CIDetectorAccuracy: CIDetectorAccuracyHigh]
                  ),
                  let ciImage = CIImage(image: qrcodeImg)
            else {
                return
            }

            var qrCodeLink = ""

            let features = detector.features(in: ciImage)

            for feature in features as! [CIQRCodeFeature] {
                qrCodeLink = feature.messageString!
                if qrCodeLink.isEmpty {
                    didFail(reason: .badOutput)
                } else {
                    let corners = [
                        feature.bottomLeft,
                        feature.bottomRight,
                        feature.topRight,
                        feature.topLeft,
                    ]
                    let result = ScanResult(
                        data: .string(qrCodeLink), type: .qr, image: qrcodeImg, corners: corners
                    )
                    found(result)
                }
            }
        }

        public func imagePickerControllerDidCancel(_: UIImagePickerController) {
            isGalleryShowing = false
            dismiss(animated: true)
        }
    }

    // MARK: - UIAdaptivePresentationControllerDelegate

    @available(macCatalyst 14.0, *)
    extension CodeScannerView.ScannerViewController: UIAdaptivePresentationControllerDelegate {
        public func presentationControllerDidDismiss(_: UIPresentationController) {
            // Gallery is no longer being presented
            isGalleryShowing = false
        }
    }

    // MARK: - AVCapturePhotoCaptureDelegate

    @available(macCatalyst 14.0, *)
    extension CodeScannerView.ScannerViewController: AVCapturePhotoCaptureDelegate {
        public func photoOutput(
            _: AVCapturePhotoOutput,
            didFinishProcessingPhoto photo: AVCapturePhoto,
            error _: Error?
        ) {
            isCapturing = false
            guard let imageData = photo.fileDataRepresentation() else {
                print("Error while generating image from photo capture data.")
                return
            }
            guard let qrImage = UIImage(data: imageData) else {
                print("Unable to generate UIImage from image data.")
                return
            }
            handler?(qrImage)
        }

        public func photoOutput(
            _: AVCapturePhotoOutput,
            willCapturePhotoFor _: AVCaptureResolvedPhotoSettings
        ) {
            AudioServicesDisposeSystemSoundID(1108)
        }

        public func photoOutput(
            _: AVCapturePhotoOutput,
            didCapturePhotoFor _: AVCaptureResolvedPhotoSettings
        ) {
            AudioServicesDisposeSystemSoundID(1108)
        }
    }

    // MARK: - AVCaptureDevice

    @available(macCatalyst 14.0, *)
    public extension AVCaptureDevice {
        /// This returns the Ultra Wide Camera on capable devices and the default Camera for Video otherwise.
        static var bestForVideo: AVCaptureDevice? {
            let deviceHasUltraWideCamera = !AVCaptureDevice.DiscoverySession(
                deviceTypes: [.builtInUltraWideCamera], mediaType: .video, position: .back
            ).devices.isEmpty
            return deviceHasUltraWideCamera
                ? AVCaptureDevice.default(.builtInUltraWideCamera, for: .video, position: .back)
                : AVCaptureDevice.default(for: .video)
        }
    }
#endif
