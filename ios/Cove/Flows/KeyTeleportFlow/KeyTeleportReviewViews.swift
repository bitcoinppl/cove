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
            Label("Recovery words received", systemImage: "key.horizontal.fill")
                .font(.headline)

            Text("Cove found a \(review.wordCount)-word wallet. Review it below or import it directly.")
                .font(.subheadline)
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

            Button("Import Wallet", action: importWords)
                .buttonStyle(OnboardingPrimaryButtonStyle())

            Button("Finish Without Importing", action: finish)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }

    private func recoveryWordsGrid(_ words: [String]) -> some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 120), spacing: 8)], spacing: 8) {
            ForEach(Array(words.enumerated()), id: \.offset) { index, word in
                HStack {
                    Text("\(index + 1)")
                        .foregroundStyle(.coveLightGray.opacity(0.6))
                    Text(word)
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
            Label("Extended private key received", systemImage: "key.horizontal.fill")
                .font(.headline)

            Text("Import this key as a hot wallet, or reveal it only when you are ready to handle the private key.")
                .font(.subheadline)
                .foregroundStyle(.coveLightGray.opacity(0.74))

            if review.revealed, let xprv {
                Text(xprv)
                    .font(.system(.caption, design: .monospaced))
                    .padding()
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.midnightBlue.opacity(0.48))
                    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

                HStack {
                    Button { keyTeleportCopySensitiveText(xprv) } label: {
                        Label("Copy", systemImage: "doc.on.doc")
                    }
                    .buttonStyle(.bordered)
                    .tint(.white)

                    Button("Hide", action: hide)
                        .buttonStyle(.bordered)
                        .tint(.white)
                }
            } else {
                Text("Reveal only if you are ready to handle this private key.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)

                Button(action: reveal) {
                    Text("Reveal XPRV")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(OnboardingSecondaryButtonStyle())
            }

            Button("Import Wallet", action: importWallet)
                .buttonStyle(OnboardingPrimaryButtonStyle())

            Button("Finish Without Importing", action: finish)
                .buttonStyle(OnboardingSecondaryButtonStyle())
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
            Label(review.items.count == 1 ? "Message received" : "Messages received", systemImage: "note.text")
                .font(.headline)

            Text("This transfer contains text, not a wallet. Cove has displayed it exactly as received.")
                .font(.subheadline)
                .foregroundStyle(.coveLightGray.opacity(0.74))

            ForEach(Array(review.items.enumerated()), id: \.offset) { _, item in
                KeyTeleportMessageItemView(item: item)
            }

            Button("Done", action: finish)
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

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Label(title, systemImage: systemImage)
                .font(.headline)

            Spacer()

            if !group.isEmpty {
                Text(group)
                    .font(.caption)
                    .foregroundStyle(.coveLightGray.opacity(0.7))
            }
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
                    .foregroundStyle(.coveLightGray.opacity(0.58))

                Text(value)
                    .font(.body)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }
}
