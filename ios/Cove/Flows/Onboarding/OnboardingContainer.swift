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
        // the lock starts locked and onboarding draws no lock screen, so with no auth set up open
        // it here as the main view does, or alerts and passkey requests would wait behind a lock
        // that never shows
        .onAppear { auth.unlockIfAuthDisabled() }
    }
}
