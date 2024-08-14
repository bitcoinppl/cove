//
//  PopupMiddleView.swift
//  Cove
//
//  Created by Praveen Perera on 7/22/24.
//

import ActivityIndicatorView
import SwiftUI

enum PopupState: Equatable {
    case initial
    case loading
    case failure(String)
    case success(String)
}

struct MiddlePopupView: View {
    var state: PopupState

    var heading: String?
    var message: String?

    var buttonText: String = "OK"

    var onClose: () -> Void = {}

    // private
    private let screenWidth = UIScreen.main.bounds.width
    private let screenHeight = UIScreen.main.bounds.height

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
            Image(systemName: "x.square.fill")
                .padding(.top, 12)
                .font(.title)
                .foregroundColor(.red)
        case .success:
            Image(systemName: "checkmark.square.fill")
                .padding(.top, 12)
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
                "Failure"
            case .success:
                "Success"
            }

        Text(heading ?? headingFromState)
            .foregroundColor(.primary)
            .font(.title)
            .padding(.top, 12)
    }

    var popupMessage: String {
        let messageFromState = switch state {
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
        VStack(spacing: 12) {
            if !isLoading {
                HStack {
                    HeadingIcon
                    Heading
                }

                Text(popupMessage)
                    .font(.subheadline)
                    .foregroundColor(.primary)
                    .opacity(0.6)
                    .padding(.top, 10)
                    .padding(.bottom, 20)

                Button {
                    onClose()
                } label: {
                    Text(buttonText)
                        .font(.title3)
                        .fontWeight(.bold)
                        .foregroundColor(Color.white)
                }
                .frame(minWidth: screenWidth * 0.6)
                .padding(.vertical, 12)
                .background(.black.opacity(0.7))
                .cornerRadius(6)
                .padding()

            } else {
                ProgressView(label: {
                    Text("Loading")
                        .font(.caption)
                        .foregroundColor(.secondary)
                })
                .progressViewStyle(.circular)
                .frame(minWidth: screenWidth * 0.65)
            }
        }
        .cornerRadius(20)
        .shadow(color: .black.opacity(0.08), radius: 2, x: 0, y: 0)
        .shadow(color: .black.opacity(0.16), radius: 24, x: 0, y: 0)
        .padding()
    }
}

#Preview("Loading") {
    VStack {
        MiddlePopupView(state: .loading)
    }
}

#Preview("Success") {
    MiddlePopupView(state: .success("Node loaded successfully"))
}

#Preview("Failure") {
    MiddlePopupView(state: .failure("Node did not load!"))
}
