import SwiftUI

struct CoveView: View {
    var model: MainViewModel

    var body: some View {
        VStack(spacing: 20) {
            Button(action: { model.pushRoute(RouteFactory().newWalletSelect()) }) {
                Text("Push Route")
            }

            Button(action: { model.setRoute([RouteFactory().newWalletSelect()]) }) {
                Text("Set Route")
            }

            Button(action: { try! model.database.toggleBoolConfig(key: GlobalBoolConfigKey.completedOnboarding) }) {
                Text("Onboarding: \(try! model.database.getBoolConfig(key: GlobalBoolConfigKey.completedOnboarding))")
            }
        }
        .padding()
        .enableInjection()
    }

    #if DEBUG
    @ObserveInjection var forceRedraw
    #endif
}

#Preview {
    CoveView(model: MainViewModel())
}
