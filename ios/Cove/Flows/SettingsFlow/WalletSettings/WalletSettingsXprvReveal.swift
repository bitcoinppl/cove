import SwiftUI
import UniformTypeIdentifiers

struct XprvRevealSheet: View {
    @Environment(\.dismiss) private var dismiss

    @Binding var xprv: String?
    @State private var copied = false

    var body: some View {
        NavigationStack {
            XprvRevealContent(xprv: $xprv, copied: $copied)
                .navigationTitle("Private Key")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    XprvRevealToolbar(done: done)
                }
        }
        .interactiveDismissDisabled()
        .onDisappear(perform: clear)
    }

    private func done() {
        clear()
        dismiss()
    }

    private func clear() {
        xprv = nil
    }
}

struct XprvRevealContent: View {
    @Binding var xprv: String?
    @Binding var copied: Bool

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                Text("Extended Private Key")
                    .font(.headline)

                if let xprv {
                    Text(xprv)
                        .font(.system(.caption, design: .monospaced))
                        .privacySensitive()
                        .padding()
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color(UIColor.secondarySystemBackground))
                        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

                    Button {
                        copySensitiveXprv(xprv)
                        copied = true
                    } label: {
                        Label(
                            copied ? "Copied for 2 Minutes" : "Copy for 2 Minutes",
                            systemImage: copied ? "checkmark" : "doc.on.doc"
                        )
                        .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                }

                Text("The clipboard copy stays on this device and expires after two minutes.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            .padding()
        }
    }
}

struct XprvRevealToolbar: ToolbarContent {
    let done: () -> Void

    var body: some ToolbarContent {
        ToolbarItem(placement: .confirmationAction) {
            Button("Done", action: done)
        }
    }
}

private func copySensitiveXprv(_ xprv: String) {
    UIPasteboard.general.setItems(
        [[UTType.utf8PlainText.identifier: xprv]],
        options: [
            .localOnly: true,
            .expirationDate: Date().addingTimeInterval(120),
        ]
    )
}
