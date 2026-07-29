import SwiftUI

struct WalletSettingsInformationSection: View {
    let metadata: WalletMetadata
    let accountNumber: UInt32?
    let masterFingerprint: String?

    var body: some View {
        Section(header: Text("Wallet Information")) {
            WalletSettingsValueRow(title: "Network", value: metadata.network.description)

            if let birthday = metadata.birthday {
                WalletSettingsValueRow(title: "Birthday", value: birthday.displayValue)
            }

            if let accountNumber {
                WalletSettingsValueRow(title: "Account Number", value: "\(accountNumber)")
            }

            if let masterFingerprint, !metadata.isTapSigner() {
                WalletSettingsValueRow(title: "Fingerprint", value: masterFingerprint)
            }

            if case let .tapSigner(tapSigner) = metadata.hardwareMetadata {
                WalletSettingsValueRow(
                    title: "Card Identifier",
                    value: tapSigner.fullCardIdent(),
                    minimumScaleFactor: 0.75
                )
            }

            WalletSettingsValueRow(title: "Wallet Type", value: String(metadata.walletType))
        }
    }
}

struct WalletSettingsValueRow: View {
    let title: String
    let value: String
    var minimumScaleFactor: CGFloat = 1

    var body: some View {
        HStack {
            Text(title)
            Spacer()
            Text(value)
                .foregroundColor(.secondary)
                .minimumScaleFactor(minimumScaleFactor)
        }
        .font(.subheadline)
    }
}

struct WalletSettingsPreferencesSection: View {
    let metadata: WalletMetadata
    let colorColumns: [GridItem]
    @Binding var showLabels: Bool
    let changeName: () -> Void
    let updateColor: (WalletColor) -> Void

    var body: some View {
        Section(header: Text("Settings")) {
            WalletSettingsNameRow(name: metadata.name, changeName: changeName)
            WalletSettingsColorPicker(
                selectedColor: metadata.color,
                colorColumns: colorColumns,
                updateColor: updateColor
            )
            Toggle(isOn: $showLabels) {
                Text("Show transaction labels")
                    .font(.subheadline)
            }
            .padding(.vertical, 1)
        }
    }
}

struct WalletSettingsNameRow: View {
    let name: String
    let changeName: () -> Void

    var body: some View {
        HStack {
            Text("Name")
            Spacer()
            Text(name)
                .font(.subheadline)
                .foregroundColor(.secondary)
            Image(systemName: "chevron.right")
                .foregroundColor(Color(UIColor.tertiaryLabel))
                .font(.footnote)
                .fontWeight(.semibold)
        }
        .contentShape(Rectangle())
        .font(.subheadline)
        .onTapGesture(perform: changeName)
    }
}

struct WalletSettingsColorPicker: View {
    let selectedColor: WalletColor
    let colorColumns: [GridItem]
    let updateColor: (WalletColor) -> Void

    var body: some View {
        VStack(spacing: 14) {
            HStack {
                Text("Wallet Color")
                    .font(.subheadline)
                Spacer()
            }
            HStack {
                Rectangle()
                    .fill(Color(selectedColor))
                    .cornerRadius(10)
                    .frame(width: 80, height: 80)
                LazyVGrid(columns: colorColumns, spacing: 20) {
                    ForEach(defaultWalletColors(), id: \.self) { color in
                        WalletSettingsColorButton(
                            color: color,
                            isSelected: color == selectedColor,
                            select: { updateColor(color) }
                        )
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                .frame(maxWidth: .infinity)
            }
        }
        .padding(.vertical, 8)
    }
}

struct WalletSettingsColorButton: View {
    let color: WalletColor
    let isSelected: Bool
    let select: () -> Void

    var body: some View {
        ZStack {
            if isSelected {
                Circle()
                    .stroke(Color(color).opacity(0.7), lineWidth: 2)
                    .frame(width: 32, height: 32)
            }
            Circle()
                .fill(Color(color))
                .frame(width: 28, height: 28)
                .contentShape(Rectangle())
        }
        .onTapGesture(perform: select)
    }
}

extension WalletBirthday {
    var displayValue: String {
        switch self {
        case .blockHeight:
            "Block \(blockHeightFmt() ?? "")"
        case let .timestamp(timestamp):
            Date(timeIntervalSince1970: TimeInterval(timestamp))
                .formatted(date: .abbreviated, time: .omitted)
        }
    }
}
