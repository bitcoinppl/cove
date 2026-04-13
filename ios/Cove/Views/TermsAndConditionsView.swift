//
//  TermsAndConditionsView.swift
//  Cove
//
//  Created by Praveen Perera on 6/3/25.
//

import SwiftUI
import UIKit

struct TermsAndConditionsView: View {
    let errorMessage: String?
    let onAgree: () -> Void

    @State private var checks: [Bool] = Array(repeating: false, count: 5)
    @Environment(\.openURL) private var openURL

    private var allChecked: Bool {
        checks.allSatisfy(\.self)
    }

    var body: some View {
        ViewThatFits(in: .vertical) {
            content(cardSpacing: 10, cardPadding: 18, footerTopSpacing: 16)
            content(cardSpacing: 8, cardPadding: 14, footerTopSpacing: 12)
        }
        .padding(.horizontal, 26)
        .padding(.top, 22)
        .padding(.bottom, 24)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .onboardingRecoveryBackground()
    }

    private func content(cardSpacing: CGFloat, cardPadding: CGFloat, footerTopSpacing: CGFloat) -> some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 12) {
                Text("Terms & Conditions")
                    .font(OnboardingRecoveryTypography.termsTitle)
                    .foregroundStyle(.white)
                    .multilineTextAlignment(.leading)
                    .frame(maxWidth: .infinity, alignment: .leading)

                Text("By continuing, you agree to the following:")
                    .font(OnboardingRecoveryTypography.subheadline)
                    .foregroundStyle(.coveLightGray.opacity(0.74))
                    .multilineTextAlignment(.leading)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }

            Spacer()
                .frame(height: 20)

            VStack(spacing: cardSpacing) {
                TermsCheckboxCard(isOn: $checks[0], cardPadding: cardPadding) {
                    Text("I understand that I am responsible for securely managing and backing up my wallets. Cove does not store or recover wallet information.")
                }

                TermsCheckboxCard(isOn: $checks[1], cardPadding: cardPadding) {
                    Text("I understand that any unlawful use of Cove is strictly prohibited.")
                }

                TermsCheckboxCard(isOn: $checks[2], cardPadding: cardPadding) {
                    Text("I understand that Cove is not a bank, exchange, or licensed financial institution, and does not offer financial services.")
                }

                TermsCheckboxCard(isOn: $checks[3], cardPadding: cardPadding) {
                    Text("I understand that if I lose access to my wallet, Cove cannot recover my funds or credentials.")
                }

                TermsCheckboxCard(isOn: $checks[4], cardPadding: cardPadding) {
                    TermsAgreementText {
                        openURL($0)
                    }
                }
            }

            Spacer()
                .frame(height: footerTopSpacing)

            if let errorMessage {
                OnboardingInlineMessage(text: errorMessage)
                    .padding(.bottom, 8)
            }

            Text("By checking these boxes, you accept and agree to the above terms.")
                .font(OnboardingRecoveryTypography.subheadline)
                .foregroundStyle(.coveLightGray.opacity(0.5))
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, 4)

            Spacer(minLength: 20)

            Button("Agree and Continue") {
                guard allChecked else { return }
                onAgree()
            }
            .buttonStyle(OnboardingPrimaryButtonStyle())
            .disabled(!allChecked)
        }
    }
}

private struct TermsAgreementText: UIViewRepresentable {
    let onOpenURL: (URL) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onOpenURL: onOpenURL)
    }

    func makeUIView(context: Context) -> LinkOnlyTextView {
        let textView = LinkOnlyTextView()
        textView.delegate = context.coordinator
        textView.isEditable = false
        textView.isSelectable = true
        textView.isScrollEnabled = false
        textView.backgroundColor = .clear
        textView.textContainerInset = .zero
        textView.textContainer.lineFragmentPadding = 0
        textView.adjustsFontForContentSizeCategory = true
        textView.showsVerticalScrollIndicator = false
        textView.showsHorizontalScrollIndicator = false
        textView.linkTextAttributes = Self.linkAttributes
        return textView
    }

    func updateUIView(_ uiView: LinkOnlyTextView, context _: Context) {
        uiView.attributedText = Self.attributedText
        uiView.linkTextAttributes = Self.linkAttributes
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: LinkOnlyTextView, context _: Context) -> CGSize? {
        guard let width = proposal.width else { return nil }
        let fittingSize = uiView.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude))
        return CGSize(width: width, height: ceil(fittingSize.height))
    }

    private static let baseFont = UIFont.preferredFont(forTextStyle: .footnote)
    private static let boldFont = UIFontMetrics(forTextStyle: .footnote).scaledFont(
        for: .systemFont(ofSize: baseFont.pointSize, weight: .bold)
    )
    private static let textColor = UIColor.white.withAlphaComponent(0.82)
    private static let linkColor = UIColor(Color.btnGradientLight)
    private static let paragraphStyle: NSParagraphStyle = {
        let style = NSMutableParagraphStyle()
        style.lineBreakMode = .byWordWrapping
        return style
    }()

    private static let bodyAttributes: [NSAttributedString.Key: Any] = [
        .font: baseFont,
        .foregroundColor: textColor,
        .paragraphStyle: paragraphStyle,
    ]
    private static let linkAttributes: [NSAttributedString.Key: Any] = [
        .font: boldFont,
        .foregroundColor: linkColor,
        .underlineStyle: NSUnderlineStyle.single.rawValue,
    ]
    private static let attributedText: NSAttributedString = {
        let text = NSMutableAttributedString(
            string: "I have read and agree to Cove’s ",
            attributes: bodyAttributes
        )

        text.append(
            NSAttributedString(
                string: "Privacy Policy",
                attributes: bodyAttributes.merging(
                    [
                        .font: boldFont,
                        .foregroundColor: linkColor,
                        .underlineStyle: NSUnderlineStyle.single.rawValue,
                        .link: URL(string: "https://covebitcoinwallet.com/privacy")!,
                    ],
                    uniquingKeysWith: { _, new in new }
                )
            )
        )
        text.append(NSAttributedString(string: " and ", attributes: bodyAttributes))
        text.append(
            NSAttributedString(
                string: "Terms & Conditions",
                attributes: bodyAttributes.merging(
                    [
                        .font: boldFont,
                        .foregroundColor: linkColor,
                        .underlineStyle: NSUnderlineStyle.single.rawValue,
                        .link: URL(string: "https://covebitcoinwallet.com/terms")!,
                    ],
                    uniquingKeysWith: { _, new in new }
                )
            )
        )
        text.append(NSAttributedString(string: " as a condition of use.", attributes: bodyAttributes))

        return text
    }()

    final class Coordinator: NSObject, UITextViewDelegate {
        private let onOpenURL: (URL) -> Void

        init(onOpenURL: @escaping (URL) -> Void) {
            self.onOpenURL = onOpenURL
        }

        func textView(
            _: UITextView,
            shouldInteractWith url: URL,
            in _: NSRange,
            interaction _: UITextItemInteraction
        ) -> Bool {
            onOpenURL(url)
            return false
        }
    }
}

private struct TermsCheckboxCard<Content: View>: View {
    @Binding var isOn: Bool
    var cardPadding: CGFloat
    var allowsCardToggle = true
    @ViewBuilder let content: () -> Content

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            Button {
                isOn.toggle()
            } label: {
                Image(systemName: isOn ? "checkmark.circle.fill" : "circle")
                    .font(.system(size: 18, weight: .medium))
                    .foregroundStyle(isOn ? Color.btnGradientLight : Color.btnGradientLight.opacity(0.92))
            }
            .buttonStyle(.plain)
            .padding(.top, 1)

            content()
                .font(OnboardingRecoveryTypography.footnote)
                .foregroundStyle(.white.opacity(0.82))
                .tint(.btnGradientLight)
                .fixedSize(horizontal: false, vertical: true)
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(Color.duskBlue.opacity(0.48))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(Color.coveLightGray.opacity(0.14), lineWidth: 1)
        )
        .contentShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        .onTapGesture {
            guard allowsCardToggle else { return }
            isOn.toggle()
        }
    }
}

private final class LinkOnlyTextView: UITextView {
    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        guard super.point(inside: point, with: event), attributedText.length > 0 else { return false }

        let textContainerPoint = CGPoint(
            x: point.x - textContainerInset.left,
            y: point.y - textContainerInset.top
        )

        let glyphIndex = layoutManager.glyphIndex(for: textContainerPoint, in: textContainer)
        let glyphRect = layoutManager.boundingRect(
            forGlyphRange: NSRange(location: glyphIndex, length: 1),
            in: textContainer
        )

        guard glyphRect.contains(textContainerPoint) else { return false }

        let characterIndex = layoutManager.characterIndexForGlyph(at: glyphIndex)
        guard characterIndex < attributedText.length else { return false }

        return attributedText.attribute(.link, at: characterIndex, effectiveRange: nil) != nil
    }
}

#Preview {
    TermsAndConditionsView(errorMessage: nil, onAgree: {})
}
