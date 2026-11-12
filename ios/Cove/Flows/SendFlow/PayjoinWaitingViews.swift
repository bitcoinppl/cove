//
//  PayjoinWaitingViews.swift
//  Cove
//

import SwiftUI

/// A Payjoin payment waiting for its receiver, with the Unix time its wait ends
struct PayjoinNegotiation: Equatable {
    let sessionId: PayjoinSessionId
    let expiresAtSecs: UInt64
}

extension PayjoinStatus {
    /// The Payjoin session this status belongs to, nil when no Payjoin payment is in progress
    var sessionId: PayjoinSessionId? {
        switch self {
        case .idle:
            nil
        case let .negotiating(sessionId: sessionId, expiresAtSecs: _):
            sessionId
        case let .broadcasting(sessionId: sessionId, txid: _, kind: _):
            sessionId
        case let .completed(sessionId: sessionId, txid: _, kind: _):
            sessionId
        }
    }

    /// The payment's wait for its receiver, nil unless it is still negotiating
    var negotiation: PayjoinNegotiation? {
        guard case let .negotiating(sessionId: sessionId, expiresAtSecs: expiresAtSecs) = self
        else { return nil }

        return PayjoinNegotiation(sessionId: sessionId, expiresAtSecs: expiresAtSecs)
    }
}

private func secondsUntil(_ expiresAtSecs: UInt64) -> Int64 {
    max(0, Int64(expiresAtSecs) - Int64(Date().timeIntervalSince1970))
}

private func formatCountdown(_ seconds: Int64) -> String {
    "\(seconds / 60):" + String(format: "%02lld", seconds % 60)
}

/// Ticks `secondsLeft` once a second until `expiresAtSecs`, a Unix time
///
/// The ticker runs in `.task`, which SwiftUI cancels when the view disappears
private struct PayjoinCountdown: ViewModifier {
    let expiresAtSecs: UInt64
    @Binding var secondsLeft: Int64

    func body(content: Content) -> some View {
        content.task(id: expiresAtSecs) {
            secondsLeft = secondsUntil(expiresAtSecs)

            while secondsLeft > 0, !Task.isCancelled {
                // wake on the next wall clock second so the display never skips a second
                let millisIntoSecond = Int(Date().timeIntervalSince1970 * 1000) % 1000
                try? await Task.sleep(for: .milliseconds(1000 - millisIntoSecond))
                if Task.isCancelled { return }

                secondsLeft = secondsUntil(expiresAtSecs)
            }
        }
    }
}

/// The countdown, or a sending note once the deadline passes or the user asked to send without
/// Payjoin
///
/// The note never claims a fallback: a proposal the receiver signed before the cancel reached the
/// wallet still goes out, and only the broadcast status says which transaction did
private struct PayjoinCountdownText: View {
    let secondsLeft: Int64
    let cancelRequested: Bool

    var body: some View {
        Group {
            if cancelRequested || secondsLeft == 0 {
                Text("Sending payment…")
            } else {
                Text("\(formatCountdown(secondsLeft)) left")
            }
        }
        .foregroundStyle(.secondary)
        .monospacedDigit()
    }
}

/// The confirm screen's waiting state while the receiver may still join its Payjoin payment
///
/// `sendWithoutPayjoin` receives the session to cancel; the wallet then broadcasts the normal
/// transaction at once. Give each session its own view identity with `.id(sessionId)`
struct PayjoinWaitingCard: View {
    let negotiation: PayjoinNegotiation
    let sendWithoutPayjoin: (PayjoinSessionId) -> Void

    @State private var secondsLeft: Int64
    @State private var cancelRequested = false

    init(
        negotiation: PayjoinNegotiation,
        sendWithoutPayjoin: @escaping (PayjoinSessionId) -> Void
    ) {
        self.negotiation = negotiation
        self.sendWithoutPayjoin = sendWithoutPayjoin
        _secondsLeft = State(initialValue: secondsUntil(negotiation.expiresAtSecs))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                ProgressView()
                    .controlSize(.small)

                Text("Waiting for the receiver")
                    .font(.subheadline)
                    .fontWeight(.semibold)

                Spacer(minLength: 8)

                PayjoinCountdownText(
                    secondsLeft: secondsLeft,
                    cancelRequested: cancelRequested
                )
                .font(.footnote)
            }

            Text(
                "The receiver can join this payment with Payjoin. If they don't in time, Cove sends it as a normal transaction."
            )
            .font(.footnote)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            Button("Send without Payjoin", action: cancel)
                .font(.subheadline)
                .fontWeight(.semibold)
                .disabled(cancelRequested || secondsLeft == 0)
                .padding(.top, 4)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.secondarySystemBackground, in: RoundedRectangle(cornerRadius: 16))
        .modifier(PayjoinCountdown(expiresAtSecs: negotiation.expiresAtSecs, secondsLeft: $secondsLeft))
    }

    private func cancel() {
        cancelRequested = true
        sendWithoutPayjoin(negotiation.sessionId)
    }
}

/// A small wallet screen notice for a Payjoin payment still in progress, such as one resumed
/// after a relaunch once its confirm screen is gone
///
/// Shows nothing when `status` is idle or completed
struct PayjoinPendingBanner: View {
    let status: PayjoinStatus
    let sendWithoutPayjoin: (PayjoinSessionId) -> Void

    var body: some View {
        switch status {
        case let .negotiating(sessionId: sessionId, expiresAtSecs: expiresAtSecs):
            PayjoinPendingNegotiationRow(
                negotiation: PayjoinNegotiation(sessionId: sessionId, expiresAtSecs: expiresAtSecs),
                sendWithoutPayjoin: sendWithoutPayjoin
            )
            .id(sessionId)

        // the broadcasting status names the transaction, so only now can it say fallback
        case let .broadcasting(sessionId: _, txid: _, kind: kind):
            HStack(spacing: 10) {
                ProgressView()
                    .controlSize(.small)

                Text(kind == .fallback ? "Sending without Payjoin…" : "Sending payment…")
                    .font(.footnote)

                Spacer()
            }
            .padding(.horizontal)
            .padding(.vertical, 14)
            .frame(maxWidth: .infinity)
            .background(Color.secondarySystemBackground)

        case .idle, .completed:
            EmptyView()
        }
    }
}

private struct PayjoinPendingNegotiationRow: View {
    let negotiation: PayjoinNegotiation
    let sendWithoutPayjoin: (PayjoinSessionId) -> Void

    @State private var secondsLeft: Int64
    @State private var cancelRequested = false

    init(
        negotiation: PayjoinNegotiation,
        sendWithoutPayjoin: @escaping (PayjoinSessionId) -> Void
    ) {
        self.negotiation = negotiation
        self.sendWithoutPayjoin = sendWithoutPayjoin
        _secondsLeft = State(initialValue: secondsUntil(negotiation.expiresAtSecs))
    }

    var body: some View {
        HStack(spacing: 10) {
            ProgressView()
                .controlSize(.small)

            VStack(alignment: .leading, spacing: 2) {
                Text("Payjoin payment waiting for the receiver")
                    .font(.footnote)
                    .fontWeight(.semibold)

                PayjoinCountdownText(
                    secondsLeft: secondsLeft,
                    cancelRequested: cancelRequested
                )
                .font(.caption)
            }

            Spacer(minLength: 8)

            Button("Send without Payjoin", action: cancel)
                .font(.footnote)
                .fontWeight(.semibold)
                .disabled(cancelRequested || secondsLeft == 0)
        }
        .padding(.horizontal)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity)
        .background(Color.secondarySystemBackground)
        .modifier(PayjoinCountdown(expiresAtSecs: negotiation.expiresAtSecs, secondsLeft: $secondsLeft))
    }

    private func cancel() {
        cancelRequested = true
        sendWithoutPayjoin(negotiation.sessionId)
    }
}

#Preview("Waiting card") {
    PayjoinWaitingCard(
        negotiation: PayjoinNegotiation(
            sessionId: "preview",
            expiresAtSecs: UInt64(Date().timeIntervalSince1970) + 300
        ),
        sendWithoutPayjoin: { _ in }
    )
    .padding()
}

#Preview("Wallet banner") {
    PayjoinPendingBanner(
        status: .negotiating(
            sessionId: "preview",
            expiresAtSecs: UInt64(Date().timeIntervalSince1970) + 300
        ),
        sendWithoutPayjoin: { _ in }
    )
}
