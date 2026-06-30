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
    let primaryButtonTitle: String

    @State private var checks: [Bool] = Array(repeating: false, count: 3)

    private var allChecked: Bool {
        checks.allSatisfy(\.self)
    }

    init(
        onEnable: @escaping () -> Void,
        onCancel: @escaping () -> Void,
        message: String?,
        isBusy: Bool,
        context: CloudBackupEnableOnboardingContext = .standard,
        primaryButtonTitle: String = "Enable Cloud Backup"
    ) {
        self.onEnable = onEnable
        self.onCancel = onCancel
        self.message = message
        self.isBusy = isBusy
        self.context = context
        self.primaryButtonTitle = primaryButtonTitle
    }

    var body: some View {
        VStack(spacing: 0) {
            CloudBackupEnableCancelButton(isBusy: isBusy, onCancel: onCancel)

            ScrollView {
                VStack(spacing: 24) {
                    Spacer().frame(height: 8)
                    CloudBackupEnableHeaderIcon()
                    CloudBackupEnableTitleSection()

                    Divider().overlay(Color.coveLightGray.opacity(0.50))
                    CloudBackupEnableInfoCard(bodyText: infoCardBody)
                    if let message {
                        OnboardingInlineMessage(text: message)
                    }
                    CloudBackupEnableCheckboxSection(
                        checks: $checks,
                        firstText: firstCheckboxText,
                        secondText: secondCheckboxText,
                        thirdText: thirdCheckboxText
                    )
                    CloudBackupEnableButton(
                        title: primaryButtonTitle,
                        allChecked: allChecked,
                        isBusy: isBusy,
                        onEnable: onEnable
                    )

                    Spacer().frame(height: 16)
                }
                .padding(.horizontal)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(CloudBackupEnableBackground())
        .allowsHitTesting(!isBusy)
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
}

#Preview {
    CloudBackupEnableOnboardingView(
        onEnable: {},
        onCancel: {},
        message: nil,
        isBusy: false
    )
}
