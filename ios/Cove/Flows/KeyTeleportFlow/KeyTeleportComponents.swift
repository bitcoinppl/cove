import SwiftUI

enum KeyTeleportMnemonicDisclosure {
    case hidden
    case revealed([String])
    case failed

    var isHidden: Bool {
        if case .hidden = self { return true }
        return false
    }

    var displayedWords: [String]? {
        switch self {
        case .hidden:
            Array(repeating: "••••••", count: 4)
        case let .revealed(words):
            words
        case .failed:
            nil
        }
    }
}

enum KeyTeleportRevealedElement {
    case qrCode
    case textCode
}

struct KeyTeleportRevealable<Content: View>: View {
    let isHidden: Bool
    let hint: String
    let blurRadius: CGFloat
    let onReveal: () -> Void
    @ViewBuilder let content: Content

    var body: some View {
        content
            .blur(radius: isHidden ? blurRadius : 0)
            .accessibilityHidden(isHidden)
            .allowsHitTesting(!isHidden)
            .overlay {
                if isHidden {
                    Button {
                        withAnimation(.easeInOut(duration: 0.2), onReveal)
                    } label: {
                        ZStack {
                            Color.clear

                            Label(hint, systemImage: "eye")
                                .font(.caption)
                                .foregroundStyle(.white)
                                .fixedSize(horizontal: true, vertical: false)
                                .padding(.horizontal, 12)
                                .padding(.vertical, 8)
                                .background(
                                    Capsule()
                                        .fill(Color.midnightBlue.opacity(0.88))
                                )
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(hint)
                }
            }
    }
}

struct KeyTeleportRevealPair<QR: View, Code: View>: View {
    let qrHint: String
    let codeHint: String
    let qr: QR
    let code: Code

    @State private var revealed: KeyTeleportRevealedElement?

    init(
        qrHint: String,
        codeHint: String,
        @ViewBuilder qr: () -> QR,
        @ViewBuilder code: () -> Code
    ) {
        self.qrHint = qrHint
        self.codeHint = codeHint
        self.qr = qr()
        self.code = code()
    }

    var body: some View {
        VStack(spacing: 18) {
            KeyTeleportRevealable(
                isHidden: revealed != .qrCode,
                hint: qrHint,
                blurRadius: 14,
                onReveal: { revealed = .qrCode }
            ) {
                qr
            }

            KeyTeleportRevealable(
                isHidden: revealed != .textCode,
                hint: codeHint,
                blurRadius: 10,
                onReveal: { revealed = .textCode }
            ) {
                code
            }
        }
    }
}

struct KeyTeleportSecureInput: View {
    @Binding var text: String
    let submit: () -> Void

    @State private var isRevealed = false

    var body: some View {
        HStack(spacing: 12) {
            Group {
                if isRevealed {
                    TextField(
                        "Teleport Password",
                        text: $text,
                        prompt: keyTeleportInputPlaceholder("Teleport Password")
                    )
                } else {
                    SecureField(
                        "Teleport Password",
                        text: $text,
                        prompt: keyTeleportInputPlaceholder("Teleport Password")
                    )
                }
            }
            .foregroundStyle(.white)
            .tint(.btnGradientLight)
            .textInputAutocapitalization(.characters)
            .autocorrectionDisabled()
            .submitLabel(.go)
            .onSubmit {
                guard !text.isEmpty else { return }

                submit()
            }

            Button {
                isRevealed.toggle()
            } label: {
                Image(systemName: isRevealed ? "eye.slash" : "eye")
                    .frame(width: 28, height: 28)
            }
            .buttonStyle(.plain)
            .foregroundStyle(.coveLightGray.opacity(0.82))
            .accessibilityLabel(isRevealed ? "Hide password" : "Show password")
        }
        .keyTeleportInputChrome()
    }
}

struct KeyTeleportCodeText: View {
    let value: String

    init(_ value: String) {
        self.value = value
    }

    var body: some View {
        Text(value)
            .font(.system(.title, design: .monospaced, weight: .semibold))
    }
}

struct KeyTeleportInputChrome: ViewModifier {
    func body(content: Content) -> some View {
        content
            .font(.body)
            .foregroundStyle(.white)
            .tint(.btnGradientLight)
            .padding(.horizontal, 16)
            .padding(.vertical, 14)
            .background(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(Color.midnightBlue.opacity(0.62))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .stroke(Color.white.opacity(0.14), lineWidth: 1)
            )
    }
}

struct KeyTeleportCardModifier: ViewModifier {
    func body(content: Content) -> some View {
        content
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 22, style: .continuous)
                    .fill(Color.duskBlue.opacity(0.58))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 22, style: .continuous)
                    .stroke(Color.coveLightGray.opacity(0.12), lineWidth: 1)
            )
    }
}

extension View {
    func keyTeleportInputChrome() -> some View {
        modifier(KeyTeleportInputChrome())
    }

    func keyTeleportCard() -> some View {
        modifier(KeyTeleportCardModifier())
    }
}

func keyTeleportInputPlaceholder(_ title: LocalizedStringKey) -> Text {
    Text(title)
        .foregroundStyle(.coveLightGray.opacity(0.58))
}
