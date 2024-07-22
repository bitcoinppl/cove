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

struct PopupMiddleView: View {
    var state: PopupState

    var heading: String?
    var message: String?

    var buttonText: String = "OK"

    var onClose: () -> Void = {}

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
                "Loading"
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
        Group {
            if case .loading = state {
                ActivityIndicatorView(isVisible: Binding.constant(true), type: .default(count: 8))
                    .frame(width: 50.0, height: 50.0)
            } else {
                VStack(spacing: 12) {
                    HStack {
                        HeadingIcon

                        Heading
                    }

                    Text(popupMessage)
                        .font(.subheadline)
                        .foregroundColor(.primary)
                        .opacity(0.6)
                        .multilineTextAlignment(.center)
                        .padding(.bottom, 20)

                    Button {
                        onClose()
                    } label: {
                        Text(buttonText)
                            .font(.title3)
                            .fontWeight(/*@START_MENU_TOKEN@*/ .bold/*@END_MENU_TOKEN@*/)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 12)
                            .foregroundColor(Color.white)
                            .background(.blue)
                            .cornerRadius(6)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .padding(EdgeInsets(top: 37, leading: 24, bottom: 40, trailing: 24))
        .background(Color.white.cornerRadius(20))
        .shadow(color: .black.opacity(0.08), radius: 2, x: 0, y: 0)
        .shadow(color: .black.opacity(0.16), radius: 24, x: 0, y: 0)
        .padding(.horizontal, 40)
    }
}

#Preview("Loading") {
    PopupMiddleView(state: .loading)
}

#Preview("Success") {
    PopupMiddleView(state: .success("Node loaded successfully"))
}

#Preview("Failure") {
    PopupMiddleView(state: .failure("Node did not load!"))
}
