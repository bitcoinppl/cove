//
//  TermsAndConditionsView.swift
//  Cove
//
//  Created by Praveen Perera on 6/3/25.
//

import SwiftUI

// MARK: - Custom Checkbox Toggle (works back to iOS 16)

struct CheckboxToggleStyle: ToggleStyle {
    @Environment(\.colorScheme) var colorScheme

    func makeBody(configuration: Configuration) -> some View {
        Button(action: { configuration.isOn.toggle() }) {
            HStack(alignment: .center, spacing: 18) {
                Image(systemName: configuration.isOn ? "checkmark.circle.fill" : "circle")
                    .font(.title3)
                    .foregroundColor(configuration.isOn ? .accentColor : .secondary)
                    .padding(.top, 2)

                configuration.label
                    .foregroundColor(.primary)
                    .font(.footnote)
                    .fontWeight(.regular)
                    .fixedSize(horizontal: false, vertical: true) // allow multiline text
            }
            .padding(.vertical, 20)
            .padding(.horizontal)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(colorScheme == .light ? Color.tertiarySystemFill : Color.systemFill)
            )
        }
        .buttonStyle(.plain)
    }
}

// MARK: - Main View

struct TermsAndConditionsView: View {
    let app: AppManager

    // Toggle state for each acknowledgement
    @State private var checks: [Bool] = Array(repeating: false, count: 5)

    private var allChecked: Bool { checks.allSatisfy(\.self) }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    // Title
                    Text("Terms & Conditions")
                        .font(.largeTitle.bold())
                        .frame(maxWidth: .infinity, alignment: .leading)

                    Divider()

                    // Subtitle
                    HStack {
                        Spacer()
                        Text("By continuing, you agree to the following")
                            .font(.subheadline)
                            .multilineTextAlignment(.center)
                        Spacer()
                    }

                    Divider()

                    // Checkboxes
                    VStack(spacing: 6) {
                        Toggle(isOn: $checks[0]) {
                            Text("I understand that I am responsible for securely managing and backing up my wallets. Cove does not store or recover wallet information.")
                        }
                        .toggleStyle(CheckboxToggleStyle())

                        Toggle(isOn: $checks[1]) {
                            Text("I understand that any unlawful use of Cove is strictly prohibited.")
                        }
                        .toggleStyle(CheckboxToggleStyle())

                        Toggle(isOn: $checks[2]) {
                            Text("I understand that Cove is not a bank, exchange, or licensed financial institution, and does not offer financial services.")
                        }
                        .toggleStyle(CheckboxToggleStyle())

                        Toggle(isOn: $checks[3]) {
                            Text("I understand that if I lose access to my wallet, Cove cannot recover my funds or credentials.")
                        }
                        .toggleStyle(CheckboxToggleStyle())

                        Toggle(isOn: $checks[4]) {
                            // Links to Privacy Policy & Terms using Markdown
                            Text("I have read and agree to Cove’s **[Privacy Policy](https://covebitcoinwallet.com/privacy)** and **[Terms & Conditions](https://covebitcoinwallet.com/terms)** as a condition of use.")
                        }
                        .toggleStyle(CheckboxToggleStyle())
                    }

                    // Footnote
                    Text("By checking these boxes, you accept and agree to the above terms.")
                        .font(.footnote)
                        .foregroundColor(.secondary)
                        .padding(.top)

                    Divider()

                    // Primary action button
                    Button("Agree and Continue") {
                        if allChecked { app.agreeToTerms() }
                    }
                    .font(.headline)
                    .fontWeight(.semibold)
                    .frame(maxWidth: .infinity)
                    .disabled(!allChecked)
                }
                .padding(24)
            }
            .background(Color.systemBackground)
        }
    }
}

// MARK: - Preview

#Preview {
    TermsAndConditionsView(app: AppManager.shared)
}
