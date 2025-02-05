import SwiftUI

struct ClearButton: ViewModifier {
    @Binding var text: String

    func body(content: Content) -> some View {
        ZStack(alignment: .trailing) {
            content

            if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Image(systemName: "multiply.circle.fill")
                        .foregroundColor(.secondary)
                }
                .padding(.trailing, 8)
                .buttonStyle(.plain)
            }
        }
    }
}
