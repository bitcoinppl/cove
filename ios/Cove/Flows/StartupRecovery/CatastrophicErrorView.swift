import SwiftUI

@_exported import CoveCore

struct CatastrophicErrorView: View {
    let onResolve: () -> Void

    @State private var showWipeConfirmation = false

    var body: some View {
        VStack(spacing: 24) {
            Spacer()

            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 64))
                .foregroundStyle(.red)

            Text("Encryption Key Error")
                .font(.title)
                .fontWeight(.bold)

            Text(
                "Your app's encryption key doesn't match the stored data. This is an unexpected error that shouldn't normally occur."
            )
            .multilineTextAlignment(.center)
            .foregroundStyle(.secondary)
            .padding(.horizontal, 32)

            Spacer()

            VStack(spacing: 16) {
                Button {
                    contactSupport()
                } label: {
                    HStack {
                        Image(systemName: "envelope")
                        Text("Contact Support")
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)

                Button(role: .destructive) {
                    showWipeConfirmation = true
                } label: {
                    Text("Wipe Local Data")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
            }

            Spacer()
        }
        .padding()
        .alert("Wipe All Local Data?", isPresented: $showWipeConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Wipe Data", role: .destructive) {
                wipeAndRestart()
            }
        } message: {
            Text(
                "This will permanently delete all wallet data on this device. Make sure you have your recovery phrases backed up. This cannot be undone."
            )
        }
    }

    private func contactSupport() {
        if let url = URL(string: "mailto:feedback@covebitcoinwallet.com") {
            UIApplication.shared.open(url)
        }
    }

    private func wipeAndRestart() {
        wipeLocalData()
        onResolve()
    }
}
