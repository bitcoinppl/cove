import SwiftUI

struct CloudBackupEnableCancelButton: View {
    let isBusy: Bool
    let onCancel: () -> Void

    var body: some View {
        HStack {
            Spacer()

            Button("Cancel", action: onCancel)
                .foregroundStyle(.white)
                .font(.headline)
                .disabled(isBusy)
        }
        .padding(.horizontal)
        .padding(.top)
    }
}

struct CloudBackupEnableHeaderIcon: View {
    var body: some View {
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
}

struct CloudBackupEnableTitleSection: View {
    var body: some View {
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
}

struct CloudBackupEnableInfoCard: View {
    let bodyText: String

    var body: some View {
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

            Text(bodyText)
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
}

struct CloudBackupEnableCheckboxSection: View {
    @Binding var checks: [Bool]
    let firstText: String
    let secondText: String
    let thirdText: String

    var body: some View {
        VStack(spacing: 6) {
            Toggle(isOn: $checks[0]) {
                Text(firstText)
            }
            .toggleStyle(DarkCheckboxToggleStyle())

            Toggle(isOn: $checks[1]) {
                Text(secondText)
            }
            .toggleStyle(DarkCheckboxToggleStyle())

            Toggle(isOn: $checks[2]) {
                Text(thirdText)
            }
            .toggleStyle(DarkCheckboxToggleStyle())
        }
    }
}

struct CloudBackupEnableButton: View {
    let title: String
    let allChecked: Bool
    let isBusy: Bool
    let onEnable: () -> Void

    var body: some View {
        Button {
            if allChecked { onEnable() }
        } label: {
            Text(title)
        }
        .buttonStyle(OnboardingPrimaryButtonStyle())
        .disabled(!allChecked || isBusy)
        .animation(.easeInOut(duration: 0.2), value: allChecked)
    }
}

struct CloudBackupEnableBackground: View {
    var body: some View {
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

struct CloudBackupEnableConfirmationView: View {
    let onContinue: () -> Void
    let onCancel: () -> Void

    var body: some View {
        VStack(spacing: 22) {
            Spacer()

            VStack(spacing: 14) {
                Image(systemName: "key.fill")
                    .font(.system(size: 42, weight: .semibold))
                    .foregroundStyle(.yellow)

                Text("Confirm your passkey")
                    .font(.title2.weight(.semibold))
                    .foregroundStyle(.white)
                    .multilineTextAlignment(.center)

                Text("Your passkey was saved. Cove needs to confirm it once before enabling Cloud Backup. If it does not appear right away, use the option to search your passkey/password manager app.")
                    .font(.body)
                    .foregroundStyle(.coveLightGray)
                    .multilineTextAlignment(.center)
            }

            VStack(spacing: 12) {
                Button("Continue", action: onContinue)
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)

                Button("Cancel", role: .cancel, action: onCancel)
                    .buttonStyle(.bordered)
                    .controlSize(.large)
            }

            Spacer()
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.midnightBlue.ignoresSafeArea())
    }
}

struct CloudBackupEnableBusyOverlay: View {
    let enableFlow: CloudBackupEnableFlow?
    var titleOverride: String?
    var subtitleOverride: String?

    private var title: String {
        if let titleOverride { return titleOverride }

        return switch enableFlow {
        case .creatingPasskey:
            "Creating your passkey..."
        case .waitingForPasskeyAvailability:
            "Checking that your passkey is available..."
        case .awaitingSavedPasskeyConfirmation:
            "Checking that your passkey is available..."
        case .confirmingSavedPasskey:
            "Confirming your passkey..."
        case .uploadingInitialBackup, .retryingUploadWithStagedMaterial:
            "Creating your encrypted backup..."
        case nil, .discoveringExistingBackup, .awaitingForceNewConfirmation, .awaitingPasskeyChoice:
            "Creating your encrypted backup..."
        }
    }

    private var subtitle: String {
        if let subtitleOverride { return subtitleOverride }

        return switch enableFlow {
        case .waitingForPasskeyAvailability, .awaitingSavedPasskeyConfirmation:
            "This can take a few seconds after saving it in your passkey/password manager app"
        default:
            "Cloud Backup will continue automatically"
        }
    }

    var body: some View {
        ZStack {
            Color.black.opacity(0.55)
                .ignoresSafeArea()

            VStack(spacing: 14) {
                ProgressView()
                    .tint(.white)
                Text(title)
                    .font(.headline)
                    .foregroundStyle(.white)
                    .multilineTextAlignment(.center)
                Text(subtitle)
                    .font(.subheadline)
                    .foregroundStyle(.coveLightGray)
                    .multilineTextAlignment(.center)
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 20)
            .frame(maxWidth: 320)
            .background(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .fill(Color.midnightBlue.opacity(0.96))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .stroke(Color.white.opacity(0.08), lineWidth: 1)
            )
            .shadow(color: .black.opacity(0.35), radius: 20, y: 10)
        }
    }
}
