//
//  ReceiveView.swift
//  Cove
//
//  Created by Praveen Perera on 8/14/24.
//

import CoreImage.CIFilterBuiltins
import MijickPopups
import SwiftUI

struct ReceiveView: View {
    @Environment(\.sizeCategory) private var sizeCategory
    @Environment(AppManager.self) private var app
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var colorScheme

    let manager: WalletManager

    private let pasteboard = UIPasteboard.general
    @State private var addressInfo: AddressInfoWithDerivation?

    @State private var timeRemainingString: String = ""
    @State private var timerTask: Task<Void, Never>? = nil

    func startTimer() {
        timerTask?.cancel()
        timerTask = Task { @MainActor in
            while !Task.isCancelled {
                guard let generatedTime = manager.addressGeneratedTime else {
                    timeRemainingString = ""
                    try? await Task.sleep(for: .seconds(1))
                    continue
                }

                let elapsed = Date().timeIntervalSince(generatedTime)
                let remaining = (5 * 60) - elapsed

                if remaining > 0 {
                    if remaining <= 60 {
                        let seconds = Int(remaining)
                        timeRemainingString = String(format: "Auto-refresh in %02d", seconds)
                    } else {
                        timeRemainingString = ""
                    }
                } else {
                    timeRemainingString = "Refreshing..."
                    nextAddressSync()
                    break
                }
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    var addressLoaded: Bool {
        addressInfo != nil
    }

    func copyText() {
        dismiss()

        if let addressInfo {
            pasteboard.string = addressInfo.addressUnformatted()
            Task { @MainActor in
                await FloaterPopup(text: "Address Copied")
                    .dismissAfter(2)
                    .present()
            }
        }
    }

    func nextAddressSync() {
        Task { await nextAddress(forceNew: true) }
    }

    func nextAddress(forceNew: Bool = false) async {
        do {
            let addressInfo = try await manager.getReceiveAddress(forceNew: forceNew)
            await MainActor.run {
                self.addressInfo = addressInfo
                self.startTimer()
            }
        } catch {
            Log.error("Unable to get next address: \(error)")
            dismiss()
            app.alertState = .init(.unableToGetAddress(error: error.localizedDescription))
        }
    }

    var body: some View {
        VStack {
            // Navigation bar substitute ("Done" button)
            HStack {
                Button("Done") { dismiss() }
                    .font(.headline)
                Spacer()
            }
            .padding([.top, .horizontal])

            Spacer(minLength: 32)

            // ----- Card -----
            DynamicHeightScrollView(idealHeight: nil) {
                VStack(spacing: 0) {
                    // Top section – QR code & title
                    VStack(spacing: 24) {
                        Text(manager.walletMetadata.name)
                            .font(.title3.weight(.semibold))
                            .foregroundStyle(.white)
                            .multilineTextAlignment(.center)

                        AddressView(addressInfo: addressInfo)

                        if let path = addressInfo?.derivationPath() {
                            Text("Derivation: \(path)")
                                .font(.footnote)
                                .foregroundStyle(.white.opacity(0.3))
                                .padding(.top, 6)
                        }
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 32)
                    .background(colorScheme == .light ? .duskBlue : .duskBlue.opacity(0.4))

                    // Bottom strip – Address text
                    VStack(alignment: .leading, spacing: 8) {
                        if let address = addressInfo {
                            Text("Wallet Address")
                                .font(.footnote.weight(.medium))
                                .foregroundStyle(.white.opacity(0.7))

                            Text(address.addressSpacedOut())
                                .font(.system(.body, design: .monospaced))
                                .foregroundStyle(.white)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .padding()
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(
                        colorScheme == .light
                            ? Color(.midnightBlue).opacity(0.95) : .midnightBlue.opacity(0.4)
                    )
                }
                .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                .padding(.horizontal)

                Spacer(minLength: 4)

                if !timeRemainingString.isEmpty, addressLoaded {
                    Text(timeRemainingString)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .padding(.top, 2)
                }

                Spacer(minLength: 12)

                // ----- Copy button -----
                Button(action: copyText) {
                    Text("Copy Address")
                        .font(.headline)
                        .frame(maxWidth: .infinity)
                        .padding()
                        .foregroundStyle(.white)
                        .background(Color.midnightBtn)
                        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
                .padding(.horizontal)

                // Secondary action
                Button("Create New Address", action: nextAddressSync)
                    .font(.footnote.weight(.semibold))
                    .padding(.top, 8)
            }
        }
        .background(Color(.systemBackground))
        .task {
            await nextAddress()
        }
        .onChange(of: manager.loadState) { _, _ in
            Task { await nextAddress() }
        }
        .onChange(of: manager.addressGeneratedTime) { _, _ in
            if addressLoaded {
                startTimer()
            }
        }
        .onDisappear {
            timerTask?.cancel()
        }
    }
}

private struct AddressView: View {
    let addressInfo: AddressInfoWithDerivation?

    func generateQRCode(from string: String) -> UIImage {
        let data = Data(string.utf8)
        let filter = CIFilter.qrCodeGenerator()
        filter.setValue(data, forKey: "inputMessage")
        filter.setValue("M", forKey: "inputCorrectionLevel")

        let transform = CGAffineTransform(scaleX: 10, y: 10)

        if let outputImage = filter.outputImage?.transformed(by: transform) {
            // Crop to content to remove default padding
            let context = CIContext()
            let cgImage = context.createCGImage(outputImage, from: outputImage.extent)!

            return UIImage(cgImage: cgImage)
        }

        return UIImage(systemName: "xmark.circle") ?? UIImage()
    }

    var body: some View {
        Group {
            if let addressInfo {
                Image(uiImage: generateQRCode(from: addressInfo.addressUnformatted()))
                    .interpolation(.none)
                    .resizable()
                    .scaledToFit()
                    .padding(8)
                    .background(Color.white)
                    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 12, style: .continuous)
                            .stroke(Color.gray.opacity(0.2), lineWidth: 1)
                    )
                    .padding(.horizontal, 16)
                    .aspectRatio(1, contentMode: .fit)
            } else {
                ProgressView(label: {
                    Text("Loading")
                        .font(.caption)
                        .foregroundColor(.white)
                })
                .tint(.white)
                .progressViewStyle(.circular)
            }
        }
    }
}

#Preview {
    AsyncPreview {
        ReceiveView(manager: WalletManager(preview: "preview_only"))
            .environment(AppManager.shared)
    }
}
