//
//  HeaderIcon.swift
//  Cove
//
//  Created by Praveen Perera on 9/17/24.
//

import SwiftUI

struct HeaderIcon: View {
    @Environment(\.colorScheme) var _colorScheme

    // passed in
    var isSent: Bool
    var isConfirmed: Bool
    var numberOfConfirmations: Int? = nil

    // private
    private let presenter = HeaderIconPresenter()
    private let screenWidth = UIScreen.main.bounds.width
    private var txnState: TransactionState {
        if isConfirmed {
            .confirmed
        } else {
            .pending
        }
    }

    private var confirmationCount: Int32 {
        if let numberOfConfirmations = numberOfConfirmations {
            return Int32(numberOfConfirmations)
        }

        if isConfirmed {
            return 5
        } else {
            return 0
        }
    }

    private var direction: TransactionDirection {
        if isSent {
            .outgoing
        } else {
            .incoming
        }
    }

    private var colorScheme: FfiColorScheme {
        .init(_colorScheme)
    }

    private var circleSize: CGFloat {
        screenWidth * 0.33
    }

    private func circleOffSet(of offset: CGFloat) -> CGFloat {
        circleSize + (offset * 20)
    }

    private var icon: String {
        switch txnState {
        case .confirmed:
            return "checkmark"
        case .pending:
            return "clock.arrow.2.circlepath"
        }
    }

    private var backgroundColor: Color {
        switch (txnState, direction, colorScheme, confirmationCount) {
        case (.pending, _, .dark, _):
            return .black
        case (.pending, _, .light, _):
            return .coolGray
        case (.confirmed, .incoming, .light, 1):
            return .green.opacity(0.33)
        case (.confirmed, .incoming, .light, 2):
            return .green.opacity(0.55)
        case (.confirmed, .incoming, .light, _):
            return .green
        case (.confirmed, .outgoing, .light, 1):
            return .black.opacity(0.33)
        case (.confirmed, .outgoing, .light, 2):
            return .black.opacity(0.55)
        case (.confirmed, .outgoing, .light, _):
            return .black
        case (.confirmed, _, .dark, _):
            return .black
        }
    }

    private var iconColor: Color {
        switch (txnState, direction, colorScheme, confirmationCount) {
        case (.confirmed, .incoming, .dark, 1):
            return .green.opacity(0.5)
        case (.confirmed, .incoming, .dark, 2):
            return .green.opacity(0.8)
        case (.confirmed, .incoming, .dark, _):
            return .green
        case (.confirmed, .incoming, .light, _):
            return .white
        case (.confirmed, .outgoing, _, 1):
            return .white.opacity(0.5)
        case (.confirmed, .outgoing, _, 2):
            return .white.opacity(0.8)
        case (.confirmed, .outgoing, _, _):
            return .white
        case (.pending, _, .light, _):
            return .black.opacity(0.5)
        case (.pending, _, .dark, _):
            return .white
        }
    }

    private func ringColor(_ ringNumber: Int) -> Color {
        presenter.ringColor(state: txnState,
                            colorScheme: colorScheme,
                            direction: direction,
                            confirmations: confirmationCount,
                            ringNumber: Int32(ringNumber))
            .toColor()
    }

    var body: some View {
        ZStack {
            Circle()
                .fill(backgroundColor)
                .frame(width: circleSize, height: circleSize)

            Circle()
                .stroke(ringColor(1), lineWidth: 1)
                .frame(width: circleOffSet(of: 1), height: circleOffSet(of: 1))
                .opacity(colorScheme == .light ? 0.44 : 0.88)

            Circle()
                .stroke(ringColor(2), lineWidth: 1)
                .frame(width: circleOffSet(of: 2), height: circleOffSet(of: 2))
                .opacity(colorScheme == .light ? 0.24 : 0.66)

            Circle()
                .stroke(ringColor(3), lineWidth: 1)
                .frame(width: circleOffSet(of: 3), height: circleOffSet(of: 3))
                .opacity(colorScheme == .light ? 0.1 : 0.33)

            Image(systemName: icon)
                .foregroundColor(iconColor)
                .font(.system(size: 62))
        }
    }
}

#Preview("sent_confirmed") {
    VStack(spacing: 30) {
        HeaderIcon(isSent: true, isConfirmed: true, numberOfConfirmations: 1)
        HeaderIcon(isSent: true, isConfirmed: true, numberOfConfirmations: 2)
        HeaderIcon(isSent: true, isConfirmed: true, numberOfConfirmations: 3)
    }
}

#Preview("incoming_confirmed") {
    VStack(spacing: 30) {
        HeaderIcon(isSent: false, isConfirmed: true, numberOfConfirmations: 1)
        HeaderIcon(isSent: false, isConfirmed: true, numberOfConfirmations: 2)
        HeaderIcon(isSent: false, isConfirmed: true, numberOfConfirmations: 3)
    }
}

#Preview("pending") {
    VStack(spacing: 30) {
        HeaderIcon(isSent: true, isConfirmed: false)
        HeaderIcon(isSent: false, isConfirmed: false)
    }
}
