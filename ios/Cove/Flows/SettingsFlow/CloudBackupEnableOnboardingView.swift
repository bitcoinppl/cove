import SwiftUI

enum CloudBackupEnableOnboardingContext {
    case standard
    case hardwareImport
}

struct CloudBackupEnableOnboardingView: View {
    let onEnable: () -> Void
    let onCancel: () -> Void
    let message: String?
    let isBusy: Bool
    let context: CloudBackupEnableOnboardingContext

    @State private var checks: [Bool] = Array(repeating: false, count: 3)

    private var allChecked: Bool {
        checks.allSatisfy(\.self)
    }

    init(
        onEnable: @escaping () -> Void,
        onCancel: @escaping () -> Void,
        message: String?,
        isBusy: Bool,
        context: CloudBackupEnableOnboardingContext = .standard
    ) {
        self.onEnable = onEnable
        self.onCancel = onCancel
        self.message = message
        self.isBusy = isBusy
        self.context = context
    }

    var body: some View {
        VStack(spacing: 0) {
            cancelButton

            ScrollView {
                VStack(spacing: 24) {
                    Spacer().frame(height: 8)
                    headerIcon
                    titleSection

                    Divider().overlay(Color.coveLightGray.opacity(0.50))
                    infoCard
                    if let message {
                        OnboardingInlineMessage(text: message)
                    }
                    checkboxSection
                    enableButton

                    Spacer().frame(height: 16)
                }
                .padding(.horizontal)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(backgroundView)
        .allowsHitTesting(!isBusy)
    }

    private var cancelButton: some View {
        HStack {
            Spacer()
            Button("Cancel") { onCancel() }
                .foregroundStyle(.white)
                .font(.headline)
                .disabled(isBusy)
        }
        .padding(.horizontal)
        .padding(.top)
    }

    private var headerIcon: some View {
        ZStack {
            Circle()
                .fill(Color.duskBlue.opacity(0.4))
                .frame(width: 100, height: 100)
                .shadow(
                    color: Color(red: 0.165, green: 0.353, blue: 0.545).opacity(0.5),
                    radius: 30
                )

            Circle()
                .stroke(
                    LinearGradient(
                        colors: [.btnGradientLight, .btnGradientDark],
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    ),
                    lineWidth: 2
                )
                .frame(width: 100, height: 100)

            Image(systemName: "icloud.and.arrow.up")
                .font(.system(size: 36, weight: .medium))
                .foregroundStyle(.white)
        }
    }

    private var titleSection: some View {
        VStack(spacing: 12) {
            HStack {
                Text("Cloud Backup")
                    .font(.system(size: 38, weight: .semibold))
                    .foregroundStyle(.white)
                Spacer()
            }

            HStack {
                Text("Cloud Backup is end-to-end encrypted before it leaves your device and stored in iCloud, secured by a passkey that only you control.")
                    .font(.footnote)
                    .foregroundStyle(.coveLightGray.opacity(0.75))
                    .fixedSize(horizontal: false, vertical: true)
                Spacer()
            }
        }
    }

    private var infoCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 12) {
                Image(systemName: "person.badge.key")
                    .font(.title3)
                    .foregroundStyle(Color.btnGradientLight)
                    .frame(width: 40, height: 40)
                    .background(Color.btnGradientLight.opacity(0.15))
                    .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))

                VStack(alignment: .leading, spacing: 4) {
                    Text("How It Works")
                        .font(.subheadline)
                        .fontWeight(.semibold)
                        .foregroundStyle(.white)

                    Text("Secured with Passkey + iCloud")
                        .font(.caption)
                        .foregroundStyle(.coveLightGray.opacity(0.75))
                }

                Spacer()
            }

            Text(infoCardBody)
                .font(.caption)
                .foregroundStyle(.coveLightGray.opacity(0.60))
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(16)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(Color.duskBlue.opacity(0.5))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .stroke(Color.coveLightGray.opacity(0.15), lineWidth: 1)
        )
    }

    private var checkboxSection: some View {
        VStack(spacing: 6) {
            Toggle(isOn: $checks[0]) {
                Text(firstCheckboxText)
            }
            .toggleStyle(DarkCheckboxToggleStyle())

            Toggle(isOn: $checks[1]) {
                Text(secondCheckboxText)
            }
            .toggleStyle(DarkCheckboxToggleStyle())

            Toggle(isOn: $checks[2]) {
                Text(thirdCheckboxText)
            }
            .toggleStyle(DarkCheckboxToggleStyle())
        }
    }

    private var infoCardBody: String {
        switch context {
        case .standard:
            "Your wallet backup is end-to-end encrypted before upload and stored in iCloud Drive. Only your passkey can decrypt it, so both are needed to restore your wallets."

        case .hardwareImport:
            "This backs up your imported hardware wallet configuration and labels in iCloud Drive, and it also enables backup for compatible wallets you create in Cove later. Your hardware wallet seed and private keys are not backed up by Cove."
        }
    }

    private var firstCheckboxText: String {
        "I understand that my passkey is required to access my Cloud Backup. I must not delete my passkey."
    }

    private var secondCheckboxText: String {
        "I understand that I need access to my iCloud account. If I lose access to my passkey or my iCloud account, my Cloud Backup won't be recoverable."
    }

    private var thirdCheckboxText: String {
        switch context {
        case .standard:
            "I understand that for maximum safety, I should still manually back up my 12 or 24 words offline on pen and paper."

        case .hardwareImport:
            "I understand that Cloud Backup does not replace the offline backup for my hardware wallet seed or recovery phrase."
        }
    }

    private var enableButton: some View {
        Button {
            if allChecked { onEnable() }
        } label: {
            Text("Enable Cloud Backup")
        }
        .buttonStyle(OnboardingPrimaryButtonStyle())
        .disabled(!allChecked || isBusy)
        .animation(.easeInOut(duration: 0.2), value: allChecked)
    }

    private var backgroundView: some View {
        ZStack {
            Color.midnightBlue

            RadialGradient(
                stops: [
                    .init(color: Color(red: 0.165, green: 0.353, blue: 0.545).opacity(0.9), location: 0),
                    .init(color: Color(red: 0.118, green: 0.227, blue: 0.361).opacity(0.4), location: 0.45),
                    .init(color: .clear, location: 0.85),
                ],
                center: .init(x: 0.35, y: 0.18),
                startRadius: 0,
                endRadius: 400
            )

            RadialGradient(
                stops: [
                    .init(color: Color(red: 0.118, green: 0.290, blue: 0.420).opacity(0.8), location: 0),
                    .init(color: .clear, location: 0.75),
                ],
                center: .init(x: 0.75, y: 0.12),
                startRadius: 0,
                endRadius: 300
            )
        }
        .ignoresSafeArea()
    }
}

// MARK: - Dark Checkbox Toggle Style

struct DarkCheckboxToggleStyle: ToggleStyle {
    func makeBody(configuration: Configuration) -> some View {
        Button(action: { configuration.isOn.toggle() }) {
            HStack(alignment: .center, spacing: 18) {
                Image(systemName: configuration.isOn ? "checkmark.circle.fill" : "circle")
                    .font(.title3)
                    .foregroundColor(configuration.isOn ? .btnGradientLight : .coveLightGray.opacity(0.5))
                    .padding(.top, 2)

                configuration.label
                    .foregroundColor(.white.opacity(0.85))
                    .font(.footnote)
                    .fontWeight(.regular)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.vertical, 20)
            .padding(.horizontal)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Color.duskBlue.opacity(0.5))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .stroke(Color.coveLightGray.opacity(0.15), lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
    }
}

#Preview {
    CloudBackupEnableOnboardingView(
        onEnable: {},
        onCancel: {},
        message: nil,
        isBusy: false
    )
}
