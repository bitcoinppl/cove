//
//  TapSignerViewComponents.swift
//  Cove
//

import SwiftUI

struct TapSignerAdaptiveLayout<Content: View>: View {
    @Environment(\.sizeCategory) private var sizeCategory

    let compactContentBottomPadding: CGFloat
    let compactSafeAreaBottomPadding: CGFloat
    let content: (Bool) -> Content

    init(
        compactContentBottomPadding: CGFloat = 0,
        compactSafeAreaBottomPadding: CGFloat = 24,
        @ViewBuilder content: @escaping (Bool) -> Content
    ) {
        self.compactContentBottomPadding = compactContentBottomPadding
        self.compactSafeAreaBottomPadding = compactSafeAreaBottomPadding
        self.content = content
    }

    var body: some View {
        GeometryReader { proxy in
            let scrollableLayout = usesCompactLayout(
                sizeCategory: sizeCategory,
                availableHeight: proxy.size.height
            )

            Group {
                if scrollableLayout {
                    ScrollView {
                        content(false)
                            .padding(.bottom, compactContentBottomPadding)
                            .frame(minHeight: proxy.size.height, maxHeight: .infinity, alignment: .top)
                            .safeAreaPadding(.bottom, compactSafeAreaBottomPadding)
                    }
                    .scrollIndicators(.hidden)
                } else {
                    content(true)
                }
            }
        }
    }
}

struct TapSignerFlexibleSpacer: View {
    let enabled: Bool

    var body: some View {
        if enabled {
            Spacer()
        }
    }
}

struct TapSignerTopActionHeader: View {
    let title: String
    let systemImage: String?
    let foregroundStyle: Color
    let action: () -> Void

    init(
        _ title: String,
        systemImage: String? = nil,
        foregroundStyle: Color = .primary,
        action: @escaping () -> Void
    ) {
        self.title = title
        self.systemImage = systemImage
        self.foregroundStyle = foregroundStyle
        self.action = action
    }

    var body: some View {
        HStack {
            Button(action: action) {
                if let systemImage {
                    Image(systemName: systemImage)
                }

                Text(title)
            }

            Spacer()
        }
        .padding(.top, 20)
        .padding(.horizontal, 10)
        .foregroundStyle(foregroundStyle)
        .fontWeight(.semibold)
    }
}

struct TapSignerPinDescription: View {
    let title: String
    let message: String

    var body: some View {
        VStack(spacing: 20) {
            Text(title)
                .font(.largeTitle)
                .fontWeight(.bold)

            Text(message)
                .font(.subheadline)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal)
    }
}

struct TapSignerPinHeader: View {
    let actionTitle: String
    let systemImage: String?
    let action: () -> Void

    init(
        actionTitle: String,
        systemImage: String? = "chevron.left",
        action: @escaping () -> Void
    ) {
        self.actionTitle = actionTitle
        self.systemImage = systemImage
        self.action = action
    }

    var body: some View {
        VStack {
            TapSignerTopActionHeader(actionTitle, systemImage: systemImage, action: action)

            Image(systemName: "lock")
                .font(.system(size: 100))
                .foregroundColor(.blue)
                .padding(.top, 22)
        }
    }
}

struct TapSignerStartingPinHeader: View {
    let action: () -> Void

    var body: some View {
        VStack {
            TapSignerTopActionHeader(
                "Back",
                systemImage: "chevron.left",
                foregroundStyle: .white,
                action: action
            )

            Image(.tapSignerCard)
                .offset(y: 10)
                .clipped()
        }
        .background(Color.duskBlue)
    }
}

struct TapSignerCvcScreen<Header: View>: View {
    @Binding var cvc: String
    let focus: FocusState<Bool>.Binding
    let spacing: CGFloat
    let header: Header
    let description: TapSignerPinDescription
    let submitTitle: String
    let errorMessage: String?
    let submitAction: () -> Void
    let isSubmitting: Bool

    init(
        cvc: Binding<String>,
        focus: FocusState<Bool>.Binding,
        spacing: CGFloat,
        header: Header,
        description: TapSignerPinDescription,
        submitTitle: String,
        errorMessage: String?,
        submitAction: @escaping () -> Void,
        isSubmitting: Bool = false
    ) {
        _cvc = cvc
        self.focus = focus
        self.spacing = spacing
        self.header = header
        self.description = description
        self.submitTitle = submitTitle
        self.errorMessage = errorMessage
        self.submitAction = submitAction
        self.isSubmitting = isSubmitting
    }

    var body: some View {
        ScrollView {
            VStack(spacing: spacing) {
                header
                description

                SecureField("Enter CVC", text: $cvc)
                    .focused(focus)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.numberPad)
                    .submitLabel(.continue)
                    .onSubmit {
                        guard !isSubmitting else { return }
                        submitAction()
                    }
                    .padding()
                    .background(Color(.systemGray6))
                    .clipShape(RoundedRectangle(cornerRadius: 10))
                    .padding(.horizontal)

                if let errorMessage {
                    Text(errorMessage)
                        .font(.footnote)
                        .foregroundStyle(.red)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal)
                }

                Button(submitTitle, action: submitAction)
                    .buttonStyle(DarkButtonStyle())
                    .padding(.horizontal)
                    .disabled(isSubmitting)

                Spacer()
            }
        }
        .scrollIndicators(.hidden)
        .navigationBarHidden(true)
    }
}

struct TapSignerResultStatus: View {
    let systemImage: String
    let color: Color
    let title: String
    let message: String
    let titleFont: Font

    init(
        systemImage: String,
        color: Color,
        title: String,
        message: String,
        titleFont: Font = .largeTitle
    ) {
        self.systemImage = systemImage
        self.color = color
        self.title = title
        self.message = message
        self.titleFont = titleFont
    }

    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: systemImage)
                .font(.system(size: 100))
                .foregroundStyle(color)
                .fontWeight(.light)

            VStack(spacing: 12) {
                Text(title)
                    .font(titleFont)
                    .fontWeight(.bold)

                Text(message)
                    .font(.subheadline)
                    .foregroundStyle(.primary.opacity(0.8))
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

struct TapSignerRetryContent: View {
    let usesFlexibleSpacing: Bool
    let title: String
    let message: String
    let cancelAction: () -> Void
    let retryAction: () -> Void

    var body: some View {
        VStack(spacing: 40) {
            TapSignerTopActionHeader("Cancel", action: cancelAction)

            TapSignerFlexibleSpacer(enabled: usesFlexibleSpacing)

            TapSignerRetryStatus(title: title, message: message)
                .padding(.horizontal)

            TapSignerFlexibleSpacer(enabled: usesFlexibleSpacing)

            VStack(spacing: 14) {
                Button("Retry", action: retryAction)
                    .buttonStyle(DarkButtonStyle())
                    .padding(.horizontal)
            }
        }
    }
}

struct TapSignerContinuationContent: View {
    let usesFlexibleSpacing: Bool
    let title: String
    let message: String
    let isSubmitting: Bool
    let cancelAction: () -> Void
    let continueAction: () -> Void

    var body: some View {
        VStack(spacing: 40) {
            TapSignerTopActionHeader("Cancel", action: cancelAction)

            TapSignerFlexibleSpacer(enabled: usesFlexibleSpacing)

            TapSignerResultStatus(
                systemImage: "arrow.forward.circle.fill",
                color: .blue,
                title: title,
                message: message
            )
            .padding(.horizontal)

            TapSignerFlexibleSpacer(enabled: usesFlexibleSpacing)

            Button(isSubmitting ? "Working…" : "Continue Setup", action: continueAction)
                .buttonStyle(DarkButtonStyle())
                .padding(.horizontal)
                .disabled(isSubmitting)
        }
    }
}

private struct TapSignerRetryStatus: View {
    let title: String
    let message: String

    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: "x.circle.fill")
                .font(.system(size: 100))
                .foregroundStyle(.red)
                .fontWeight(.light)

            Text(title)
                .font(.title)
                .fontWeight(.bold)

            Text(message)
                .font(.subheadline)
                .foregroundStyle(.primary.opacity(0.8))
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

struct TapSignerSuccessContent: View {
    let usesFlexibleSpacing: Bool
    let title: String
    let message: String
    let cancelAction: () -> Void
    let continueAction: () -> Void

    var body: some View {
        VStack(spacing: 40) {
            TapSignerTopActionHeader("Cancel", action: cancelAction)

            TapSignerFlexibleSpacer(enabled: usesFlexibleSpacing)

            TapSignerResultStatus(
                systemImage: "checkmark.circle.fill",
                color: .green,
                title: title,
                message: message
            )

            TapSignerFlexibleSpacer(enabled: usesFlexibleSpacing)

            VStack(spacing: 14) {
                Button("Continue", action: continueAction)
                    .buttonStyle(DarkButtonStyle())
            }
        }
        .padding(.horizontal)
    }
}
