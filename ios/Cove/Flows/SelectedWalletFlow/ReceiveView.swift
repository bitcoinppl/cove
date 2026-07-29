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
    @Environment(AppManager.self) private var app
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var colorScheme

    let manager: WalletManager

    private let pasteboard = UIPasteboard.general
    @State private var showPaidCopyConfirmation = false

    private var receiveState: ReceiveAddressState? {
        manager.receiveAddressState
    }

    private var addressInfo: AddressInfoWithDerivation? {
        receiveState?.address
    }

    private var presentation: ReceiveAddressPresentation {
        manager.receiveAddressPresentation
    }

    private var addressLoaded: Bool {
        addressInfo != nil
    }

    var body: some View {
        ReceiveViewContent(
            walletName: manager.walletMetadata.name,
            receiveState: receiveState,
            addressInfo: addressInfo,
            presentation: presentation,
            colorScheme: colorScheme,
            addressLoaded: addressLoaded,
            isLoading: manager.receiveAddressIsLoading,
            dismiss: dismiss.callAsFunction,
            copyAddress: copyText,
            createNewAddress: nextAddressSync
        )
        .background(Color(.systemBackground))
        .task {
            await openReceiveAddress()
        }
        .onDisappear(perform: closeReceiveAddress)
        .onChange(of: manager.receiveAddressError, handleReceiveAddressError)
        .modifier(
            PaidAddressCopyConfirmationModifier(
                isPresented: $showPaidCopyConfirmation,
                createNewAddress: nextAddressSync,
                copyAnyway: copyVisibleAddressAndDismiss
            )
        )
    }

    private func copyText() {
        if presentation.copyPolicy == .confirmPaidAddress {
            showPaidCopyConfirmation = true
            return
        }

        copyVisibleAddressAndDismiss()
    }

    private func copyVisibleAddressAndDismiss() {
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

    private func nextAddressSync() {
        manager.dispatch(.createNewReceiveAddress)
    }

    private func openReceiveAddress() async {
        manager.dispatch(.openReceiveAddress)
    }

    private func closeReceiveAddress() {
        guard let requestId = receiveState?.requestId else { return }

        manager.dispatch(.closeReceiveAddress(requestId))
    }

    private func handleReceiveAddressError(
        _: TaggedItem<String>?,
        _ error: TaggedItem<String>?
    ) {
        guard let error else { return }

        Log.error("Unable to update receive address: \(error.value)")
        if !addressLoaded {
            dismiss()
        }
        app.alertState = .init(.unableToGetAddress(error: error.value))
    }
}

private struct ReceiveViewContent: View {
    let walletName: String
    let receiveState: ReceiveAddressState?
    let addressInfo: AddressInfoWithDerivation?
    let presentation: ReceiveAddressPresentation
    let colorScheme: ColorScheme
    let addressLoaded: Bool
    let isLoading: Bool
    let dismiss: () -> Void
    let copyAddress: () -> Void
    let createNewAddress: () -> Void

    var body: some View {
        VStack {
            ReceiveNavigationBar(dismiss: dismiss)

            Spacer(minLength: 32)

            DynamicHeightScrollView(idealHeight: nil) {
                ReceiveAddressCard(
                    walletName: walletName,
                    receiveState: receiveState,
                    addressInfo: addressInfo,
                    presentation: presentation,
                    colorScheme: colorScheme,
                    addressLoaded: addressLoaded
                )

                Spacer(minLength: 32)

                ReceiveCopyAddressButton(
                    addressLoaded: addressLoaded,
                    isLoading: isLoading,
                    copyAddress: copyAddress
                )

                ReceiveNewAddressButton(
                    isLoading: isLoading,
                    createNewAddress: createNewAddress
                )
            }
        }
    }
}

private struct ReceiveNavigationBar: View {
    let dismiss: () -> Void

    var body: some View {
        HStack {
            Button("Done", action: dismiss)
                .font(.headline)
            Spacer()
        }
        .padding([.top, .horizontal])
    }
}

private struct ReceiveAddressCard: View {
    let walletName: String
    let receiveState: ReceiveAddressState?
    let addressInfo: AddressInfoWithDerivation?
    let presentation: ReceiveAddressPresentation
    let colorScheme: ColorScheme
    let addressLoaded: Bool

    var body: some View {
        VStack(spacing: 0) {
            ReceiveAddressCardTop(
                walletName: walletName,
                receiveState: receiveState,
                addressInfo: addressInfo,
                refreshState: presentation.refreshState,
                colorScheme: colorScheme
            )

            ReceiveAddressCardBottom(
                addressInfo: addressInfo,
                refreshState: presentation.refreshState,
                colorScheme: colorScheme,
                addressLoaded: addressLoaded
            )
        }
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        .padding(.horizontal)
    }
}

private struct ReceiveAddressCardTop: View {
    let walletName: String
    let receiveState: ReceiveAddressState?
    let addressInfo: AddressInfoWithDerivation?
    let refreshState: ReceiveAddressRefreshState
    let colorScheme: ColorScheme

    var body: some View {
        VStack(spacing: 24) {
            Text(walletName)
                .font(.title3.weight(.semibold))
                .foregroundStyle(.white)
                .multilineTextAlignment(.center)

            AddressView(addressInfo: addressInfo)

            if receiveState?.status == .paymentReceived {
                Text("Payment Received")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.white)
            } else if refreshState == .refreshing {
                Text("Refreshing...")
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
    }
}

private struct ReceiveAddressCardBottom: View {
    let addressInfo: AddressInfoWithDerivation?
    let refreshState: ReceiveAddressRefreshState
    let colorScheme: ColorScheme
    let addressLoaded: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let address = addressInfo {
                Text("Wallet Address")
                    .font(.footnote.weight(.medium))
                    .foregroundStyle(.white.opacity(0.7))

                Text(address.addressSpacedOut())
                    .font(.system(.body, design: .monospaced))
                    .foregroundStyle(.white)
                    .fixedSize(horizontal: false, vertical: true)

                if addressLoaded, refreshState == .failed {
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
}

private struct ReceiveCopyAddressButton: View {
    let addressLoaded: Bool
    let isLoading: Bool
    let copyAddress: () -> Void

    var body: some View {
        Button(action: copyAddress) {
            Text("Copy Address")
                .font(.headline)
                .frame(maxWidth: .infinity)
                .padding()
                .foregroundStyle(.white)
                .background(Color.midnightBtn)
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        }
        .padding(.horizontal)
        .disabled(!addressLoaded || isLoading)
    }
}

private struct ReceiveNewAddressButton: View {
    let isLoading: Bool
    let createNewAddress: () -> Void

    var body: some View {
        Button("Create New Address", action: createNewAddress)
            .font(.headline.weight(.semibold))
            .padding(.top, 8)
            .disabled(isLoading)
    }
}

private struct PaidAddressCopyConfirmationModifier: ViewModifier {
    @Binding var isPresented: Bool
    let createNewAddress: () -> Void
    let copyAnyway: () -> Void

    func body(content: Content) -> some View {
        content
            .alert("Copy paid address?", isPresented: $isPresented) {
                Button("Create New Address", role: .cancel, action: createNewAddress)
                Button("Copy Anyway", role: .destructive, action: copyAnyway)
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

#Preview {
    AsyncPreview {
        ReceiveView(manager: WalletManager(preview: "preview_only"))
            .environment(AppManager.shared)
    }
}
