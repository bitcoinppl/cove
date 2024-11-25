//
//  SendFlowBbqrExport.swift
//  Cove
//
//  Created by Praveen Perera on 11/24/24.
//
import SwiftUI

struct SendFlowBbqrExport: View {
    // args
    let qrs: [QrCodeView]

    let startedAt: Date = .now

    // private
    @State private var currentIndex = 0

    var body: some View {
        VStack {
            Text("Scan this QR")
                .font(.headline)

            Text("Scan this BBQr with your hardware wallet to sign your transaction")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.top, 2)
                .padding(.horizontal, 40)

            TimelineView(.periodic(from: startedAt, by: 0.2)) { context in
                let index = abs(Int(context.date.distance(to: startedAt) / 0.2) % qrs.count)
                qrs[index]
                    .onChange(of: index) { _, newIndex in
                        currentIndex = newIndex
                    }
            }

            if qrs.count > 1 {
                HStack(spacing: 4) {
                    ForEach(0 ..< qrs.count, id: \.self) { index in
                        Rectangle()
                            .fill(Color.blue)
                            .opacity(index == currentIndex ? 1 : 0.3)
                            .frame(height: 12)
                            .cornerRadius(2)
                    }
                }
                .padding(.top, 20)
            }
        }
    }
}

#Preview {
    SendFlowBbqrExport(qrs: [
        QrCodeView(text: "hello"),
        QrCodeView(text: "world"),
        QrCodeView(text: "signal"),
        QrCodeView(text: "baby"),
        QrCodeView(text: "speaker")
    ])
    .padding()
}
