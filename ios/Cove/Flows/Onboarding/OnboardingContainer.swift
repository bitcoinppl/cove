import SwiftUI

struct OnboardingContainer: View {
    let manager: OnboardingManager
    let auth: AuthManager
    let onComplete: () -> Void

    var body: some View {
        CloudBackupPresentationHost(
            app: manager.app,
            auth: auth,
            isCoverPresented: false,
            presentationPolicy: .onboarding
        ) {
            OnboardingStepView(manager: manager)
                .onChange(of: manager.isComplete) { _, complete in
                    guard complete else { return }

                    manager.app.reloadWallets()
                    onComplete()
                }
        }
        .environment(manager.app)
    }
}
