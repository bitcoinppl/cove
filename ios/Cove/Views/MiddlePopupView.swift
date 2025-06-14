//
//  MiddlePopupView.swift
//  Cove
//
//  Created by Praveen Perera on 7/22/24.
//

import SwiftUI

enum PopupState: Equatable {
    case initial
    case loading
    case failure(String)
    case success(String)
}

struct MiddlePopupView: View {
    var state: PopupState
    var dismiss: () -> Void = {}

    var heading: String?
    var message: String?

    var buttonText: String = "OK"

    var onClose: () -> Void = {}

    // private
    private let screenWidth = UIScreen.main.bounds.width
    private let screenHeight = UIScreen.main.bounds.height
    @Environment(\.colorScheme) private var colorScheme

    var isLoading: Bool {
        state == .loading
    }

    @ViewBuilder
    var HeadingIcon: some View {
        switch state {
        case .initial:
            EmptyView()
        case .loading:
            EmptyView()
        case .failure:
            Image(systemName: "x.circle.fill")
                .font(.title)
                .foregroundColor(.red)
        case .success:
            Image(systemName: "checkmark.circle.fill")
                .font(.title)
                .foregroundColor(.green)
        }
    }

    @ViewBuilder
    var Heading: some View {
        let headingFromState =
            switch state {
            case .initial:
                ""
            case .loading:
                ""
            case .failure:
                "Something went wrong"
            case .success:
                "You're all set"
            }

        Text(heading ?? headingFromState)
            .foregroundColor(.primary)
            .font(.headline)
    }

    var popupMessage: String {
        let messageFromState =
            switch state {
            case .initial:
                ""
            case .loading:
                ""
            case let .failure(string):
                string
            case let .success(string):
                string
            }

        return message ?? messageFromState
    }

    var body: some View {
        VStack(spacing: 16) {
            if !isLoading {
                VStack(spacing: 12) {
                    HeadingIcon
                    Heading
                }

                Text(popupMessage)
                    .font(.footnote)
                    .fontWeight(.regular)
                    .foregroundColor(.primary)
                    .multilineTextAlignment(.center)

                Button {
                    dismiss()
                    onClose()
                } label: {
                    Text(buttonText)
                        .font(.caption)
                        .fontWeight(.semibold)
                        .foregroundColor(Color.white)
                        .padding(.vertical, 10)
                        .frame(minWidth: screenWidth * 0.5)
                }
                .background(.midnightBtn)
                .cornerRadius(10)
                .frame(minWidth: screenWidth * 0.62)

            } else {
                ProgressView(label: {
                    Text("Working on it...")
                        .font(.caption)
                        .padding(.vertical)

                })
                .progressViewStyle(.circular)
                .frame(minWidth: screenWidth * 0.65, minHeight: screenHeight * 0.1)
                .tint(.primary)
            }
        }
        .padding(4)
        .shadow(color: .black.opacity(0.08), radius: 2, x: 0, y: 0)
        .shadow(color: .black.opacity(0.16), radius: 24, x: 0, y: 0)
        .padding(18)
        .background(.coveBg)
        .cornerRadius(10)
    }
}

#Preview("Loading") {
    VStack {
        MiddlePopupView(state: .loading)
    }
    .frame(maxWidth: .infinity, maxHeight: .infinity)
    .background(.gray)
}

#Preview("Success") {
    VStack {
        MiddlePopupView(state: .success("Node loaded successfully"))
    }
    .frame(maxWidth: .infinity, maxHeight: .infinity)
    .background(.gray)
}

#Preview("Failure") {
    VStack {
        MiddlePopupView(state: .failure("Node did not load!"))
    }
    .frame(maxWidth: .infinity, maxHeight: .infinity)
    .background(.gray)
}
