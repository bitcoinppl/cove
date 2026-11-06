import SwiftUI
import UniformTypeIdentifiers

struct KeyTeleportMnemonicReviewView: View {
    let review: KeyTeleportMnemonicReview
    let disclosure: KeyTeleportMnemonicDisclosure
    let reveal: () -> Void
    let importWords: () -> Void
    let finish: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            KeyTeleportReviewTitle("Recovery words received", systemImage: "key.horizontal.fill")

            Text("Cove found a \(review.wordCount)-word wallet. Review it below or import it directly.")
                .font(.subheadline)
                .uiTestTruncationProbe()
                .foregroundStyle(.coveLightGray.opacity(0.74))

            if let words = disclosure.displayedWords {
                KeyTeleportRevealable(
                    isHidden: disclosure.isHidden,
                    hint: "Tap to reveal recovery words",
                    blurRadius: 10,
                    onReveal: reveal
                ) {
                    recoveryWordsGrid(words)
                }
            } else {
                Text("Unable to reveal recovery words.")
                    .foregroundStyle(.red)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 24)
            }

            KeyTeleportImportChoices(importWallet: importWords, finish: finish)
        }
    }

    private func recoveryWordsGrid(_ words: [String]) -> some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 120), spacing: 8)], spacing: 8) {
            ForEach(Array(words.enumerated()), id: \.offset) { index, word in
                HStack {
                    Text("\(index + 1)")
                        .uiTestTruncationProbe()
                        .foregroundStyle(.coveLightGray.opacity(0.6))
                    Text(word).uiTestTruncationProbe()
                    Spacer()
                }
                .font(.system(.subheadline, design: .monospaced))
                .padding(10)
                .background(Color.midnightBlue.opacity(0.48))
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
            }
        }
    }
}

struct KeyTeleportXprvReviewView: View {
    let review: KeyTeleportXprvReview
    @Binding var xprv: String?
    let reveal: () -> Void
    let hide: () -> Void
    let importWallet: () -> Void
    let finish: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            KeyTeleportReviewTitle("Extended private key received", systemImage: "key.horizontal.fill")

            Text("Import this key as a hot wallet, or reveal it only when you are ready to handle the private key.")
                .font(.subheadline)
                .uiTestTruncationProbe()
                .foregroundStyle(.coveLightGray.opacity(0.74))

            if review.revealed, let xprv {
                KeyTeleportRevealedXprv(xprv: xprv, hide: hide)
            } else {
                Text("Reveal only if you are ready to handle this private key.")
                    .font(.subheadline)
                    .uiTestTruncationProbe()
                    .foregroundStyle(.secondary)

                Button(action: reveal) {
                    Text("Reveal XPRV")
                        .uiTestTruncationProbe()
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(OnboardingSecondaryButtonStyle())
            }

            KeyTeleportImportChoices(importWallet: importWallet, finish: finish)
        }
    }
}

/// A review's title, whose text carries the truncation probe
struct KeyTeleportReviewTitle: View {
    let title: LocalizedStringKey
    let systemImage: String

    init(_ title: LocalizedStringKey, systemImage: String) {
        self.title = title
        self.systemImage = systemImage
    }

    var body: some View {
        Label {
            Text(title).uiTestTruncationProbe()
        } icon: {
            Image(systemName: systemImage)
        }
        .font(.headline)
    }
}

/// The two ways out of a wallet review: import the wallet, or end the transfer without it
struct KeyTeleportImportChoices: View {
    let importWallet: () -> Void
    let finish: () -> Void

    var body: some View {
        Button(action: importWallet) {
            Text("Import Wallet").uiTestTruncationProbe()
        }
        .buttonStyle(OnboardingPrimaryButtonStyle())

        Button(action: finish) {
            Text("Finish Without Importing").uiTestTruncationProbe()
        }
        .buttonStyle(OnboardingSecondaryButtonStyle())
    }
}

struct KeyTeleportRevealedXprv: View {
    let xprv: String
    let hide: () -> Void

    var body: some View {
        Text(xprv)
            .font(.system(.caption, design: .monospaced))
            .uiTestTruncationProbe()
            .padding()
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.midnightBlue.opacity(0.48))
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

        HStack {
            Button { keyTeleportCopySensitiveText(xprv) } label: {
                Label {
                    Text("Copy").uiTestTruncationProbe()
                } icon: {
                    Image(systemName: "doc.on.doc")
                }
            }
            .buttonStyle(.bordered)
            .tint(.white)

            Button(action: hide) {
                Text("Hide").uiTestTruncationProbe()
            }
            .buttonStyle(.bordered)
            .tint(.white)
        }
    }
}

private func keyTeleportCopySensitiveText(_ text: String) {
    UIPasteboard.general.setItems(
        [[UTType.utf8PlainText.identifier: text]],
        options: [
            .localOnly: true,
            .expirationDate: Date().addingTimeInterval(120),
        ]
    )
}

struct KeyTeleportMessageReviewView: View {
    let review: KeyTeleportMessageReview
    let finish: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            KeyTeleportReviewTitle(review.items.count == 1 ? "Message received" : "Messages received", systemImage: "note.text")

            Text("This transfer contains text, not a wallet. Cove has displayed it exactly as received.")
                .font(.subheadline)
                .uiTestTruncationProbe()
                .foregroundStyle(.coveLightGray.opacity(0.74))

            ForEach(Array(review.items.enumerated()), id: \.offset) { _, item in
                KeyTeleportMessageItemView(item: item)
            }

            Button(action: finish) {
                Text("Done").uiTestTruncationProbe()
            }
            .buttonStyle(OnboardingPrimaryButtonStyle())
        }
    }
}

struct KeyTeleportMessageItemView: View {
    let item: KeyTeleportMessageItem

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            switch item {
            case let .note(title, text, group):
                KeyTeleportMessageHeader(
                    title: title,
                    group: group,
                    systemImage: "note.text"
                )
                KeyTeleportMessageField(label: "Message", value: text)
            case let .password(title, username, password, site, notes, group):
                KeyTeleportMessageHeader(
                    title: title,
                    group: group,
                    systemImage: "lock.fill"
                )
                KeyTeleportMessageField(label: "Username", value: username)
                KeyTeleportMessageField(label: "Password", value: password)
                KeyTeleportMessageField(label: "Website", value: site)
                KeyTeleportMessageField(label: "Notes", value: notes)
            }
        }
        .padding(16)
        .background(Color.midnightBlue.opacity(0.48))
        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
    }
}

struct KeyTeleportMessageHeader: View {
    let title: String
    let group: String
    let systemImage: String

    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        // at accessibility sizes the group beside the title left it room for a few letters a
        // line, so a title such as "Exchange" broke mid-word; the group goes under it instead
        if dynamicTypeSize.isAccessibilitySize {
            VStack(alignment: .leading, spacing: 4) {
                titleLabel
                groupText
            }
        } else {
            HStack(alignment: .firstTextBaseline) {
                titleLabel
                Spacer()
                groupText
            }
        }
    }

    private var titleLabel: some View {
        Label {
            Text(title).uiTestTruncationProbe()
        } icon: {
            Image(systemName: systemImage)
        }
        .font(.headline)
    }

    @ViewBuilder
    private var groupText: some View {
        if !group.isEmpty {
            Text(group)
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(.coveLightGray.opacity(0.7))
        }
    }
}

struct KeyTeleportMessageField: View {
    let label: String
    let value: String

    var body: some View {
        if !value.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text(label.uppercased())
                    .font(.caption2.weight(.semibold))
                    .uiTestTruncationProbe()
                    .foregroundStyle(.coveLightGray.opacity(0.58))

                Text(value)
                    .font(.body)
                    .uiTestTruncationProbe()
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }
}
