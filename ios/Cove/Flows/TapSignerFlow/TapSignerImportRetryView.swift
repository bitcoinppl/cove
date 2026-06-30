//
//  TapSignerImportRetryView.swift
//  Cove
//
//  Created by Praveen Perera on 3/25/25.
//

import SwiftUI
import UniformTypeIdentifiers

struct TapSignerImportRetry: View {
    @Environment(\.sizeCategory) private var sizeCategory
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner

    var body: some View {
        GeometryReader { proxy in
            let scrollableLayout = usesScrollableLayout(availableHeight: proxy.size.height)

            Group {
                if scrollableLayout {
                    ScrollView {
                        mainContent(usesFlexibleSpacing: false)
                            .frame(minHeight: proxy.size.height, maxHeight: .infinity, alignment: .top)
                            .safeAreaPadding(.bottom, 24)
                    }
                    .scrollIndicators(.hidden)
                } else {
                    mainContent(usesFlexibleSpacing: true)
                }
            }
        }
        .background(backgroundView)
        .scrollIndicators(.hidden)
        .navigationBarHidden(true)
    }

    private func mainContent(usesFlexibleSpacing: Bool) -> some View {
        VStack(spacing: 40) {
            VStack {
                HStack {
                    Button(action: { manager.popRoute() }) {
                        Text("Cancel")
                    }

                    Spacer()
                }
                .padding(.top, 20)
                .padding(.horizontal, 10)
                .foregroundStyle(.primary)
                .fontWeight(.semibold)
            }

            if usesFlexibleSpacing {
                Spacer()
            }

            VStack(spacing: 20) {
                Image(systemName: "x.circle.fill")
                    .font(.system(size: 100))
                    .foregroundStyle(.red)
                    .fontWeight(.light)

                Text("Could not complete import")
                    .font(.title)
                    .fontWeight(.bold)

                Text(
                    "Please try again and hold your TAPSIGNER steady until import is complete."
                )
                .font(.subheadline)
                .foregroundStyle(.primary.opacity(0.8))
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal)

            if usesFlexibleSpacing {
                Spacer()
            }

            VStack(spacing: 14) {
                Button("Retry") {
                    guard let pin = manager.enteredPin else {
                        app.alertState = .init(.tapSignerDeriveFailed(message: "No PIN entered"))
                        return
                    }

                    let nfc = manager.getOrCreateNfc(tapSigner)

                    Task {
                        switch await nfc.derive(pin: pin) {
                        case let .success(deriveInfo):
                            manager.resetRoute(to: .importSuccess(tapSigner, deriveInfo))
                        case let .failure(error):
                            app.alertState = .init(.tapSignerDeriveFailed(message: error.description))
                        }
                    }
                }
                .buttonStyle(DarkButtonStyle())
                .padding(.horizontal)
            }
        }
    }

    private var backgroundView: some View {
        VStack {
            Image(.chainCodePattern)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .ignoresSafeArea(edges: .all)
                .padding(.top, 5)

            Spacer()
        }
        .opacity(0.8)
    }

    private func usesScrollableLayout(availableHeight: CGFloat) -> Bool {
        sizeCategory >= .extraExtraLarge || availableHeight <= 812
    }
}

#Preview {
    TapSignerContainer(
        route: .importRetry(tapSignerPreviewNew(preview: true))
    )
    .environment(AppManager.shared)
}
