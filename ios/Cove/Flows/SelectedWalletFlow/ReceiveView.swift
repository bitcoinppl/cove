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
    @State private var isLoading = true
    @State private var isRefreshing = false
    @State private var now = Date()
    @State private var showPaidCopyConfirmation = false
    @State private var createAddressTask: Task<Void, Never>?

    private var receiveState: ReceiveAddressState? {
        manager.receiveAddressState
    }

    private var addressInfo: AddressInfoWithDerivation? {
        receiveState?.address
    }

    private var presentation: ReceiveAddressPresentation {
        receiveAddressPresentation(
            state: receiveState,
            nowSecs: UInt64(now.timeIntervalSince1970),
            isRefreshing: isRefreshing
        )
    }

    var addressLoaded: Bool {
        addressInfo != nil
    }

    func copyText() {
        if presentation.copyPolicy == .confirmPaidAddress {
            showPaidCopyConfirmation = true
            return
        }

        copyVisibleAddressAndDismiss()
    }

    func copyVisibleAddressAndDismiss() {
        if let addressInfo {
            pasteboard.string = addressInfo.addressUnformatted()
            Task { @MainActor in
                await FloaterPopup(text: "Address Copied")
                    .dismissAfter(2)
                    .present()
            }
        }

        dismiss()
    }

    func nextAddressSync() {
        createAddressTask?.cancel()
        createAddressTask = Task { await createNewAddress() }
    }

    func openReceiveAddress() async {
        do {
            let state = try await manager.rust.openReceiveAddress()
            guard !Task.isCancelled else { return }

            await MainActor.run {
                manager.receiveAddressState = state
                isLoading = false
            }
        } catch is CancellationError {
            return
        } catch {
            guard !Task.isCancelled else { return }

            Log.error("Unable to get next address: \(error)")
            dismiss()
            app.alertState = .init(.unableToGetAddress(error: error.localizedDescription))
        }
    }

    func createNewAddress() async {
        do {
            await MainActor.run {
                isLoading = true
                isRefreshing = false
            }
            let state = try await manager.rust.createNewReceiveAddress()
            guard !Task.isCancelled else { return }

            await MainActor.run {
                manager.receiveAddressState = state
                isLoading = false
            }
        } catch is CancellationError {
            return
        } catch {
            guard !Task.isCancelled else { return }

            await MainActor.run { isLoading = false }
            Log.error("Unable to create new receive address: \(error)")
            app.alertState = .init(.unableToGetAddress(error: error.localizedDescription))
        }
    }

    func refreshExpiredAddress(requestId: UInt64) async {
        guard !isRefreshing else { return }

        do {
            await MainActor.run { isRefreshing = true }
            let state = try await manager.rust.refreshExpiredReceiveAddress(requestId: requestId)
            guard !Task.isCancelled else { return }

            await MainActor.run {
                manager.receiveAddressState = state
                isRefreshing = false
            }
        } catch is CancellationError {
            return
        } catch {
            guard !Task.isCancelled else { return }

            await MainActor.run { isRefreshing = false }
            Log.error("Unable to refresh receive address: \(error)")
        }
    }

    func closeReceiveAddress() {
        guard let requestId = receiveState?.requestId else { return }

        Task { await manager.rust.closeReceiveAddress(requestId: requestId) }
    }

    var countdownText: String? {
        if presentation.showRefreshing {
            return "Refreshing..."
        }

        guard let remaining = presentation.countdownRemainingSecs else { return nil }

        let minutes = Int(remaining / 60)
        let seconds = Int(remaining % 60)
        return "Auto-refresh in \(String(format: "%02d", minutes)):\(String(format: "%02d", seconds))"
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

                        if receiveState?.status == .paymentReceived {
                            Text("Payment Received")
                                .font(.footnote.weight(.semibold))
                                .foregroundStyle(.white)
                        } else if let countdownText {
                            Text(countdownText)
                                .font(.footnote)
                                .foregroundStyle(.white.opacity(0.65))
                        }

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

                            if addressLoaded, presentation.showRefreshError {
                                Text("Unable to refresh address")
                                    .font(.footnote)
                                    .foregroundStyle(.white.opacity(0.65))
                                    .padding(.top, 4)
                            }
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

                Spacer(minLength: 32)

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
                .disabled(!addressLoaded)

                // Secondary action
                Button("Create New Address", action: nextAddressSync)
                    .font(.headline.weight(.semibold))
                    .padding(.top, 8)
            }
        }
        .background(Color(.systemBackground))
        .task {
            await openReceiveAddress()
        }
        .task(id: receiveState?.requestId) {
            while !Task.isCancelled {
                await MainActor.run { now = Date() }

                if let receiveState, presentation.shouldRefreshNow {
                    await refreshExpiredAddress(requestId: receiveState.requestId)
                }

                try? await Task.sleep(for: .seconds(1))
            }
        }
        .onDisappear {
            createAddressTask?.cancel()
            createAddressTask = nil
            closeReceiveAddress()
        }
        .alert("Copy paid address?", isPresented: $showPaidCopyConfirmation) {
            Button("Create New Address", role: .cancel) {
                nextAddressSync()
            }
            Button("Copy Anyway", role: .destructive) {
                copyVisibleAddressAndDismiss()
            }
        } message: {
            Text("This address has already received funds. For better privacy, create a new address before sharing.")
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
