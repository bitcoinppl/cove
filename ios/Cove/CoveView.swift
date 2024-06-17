import SwiftUI

struct CoveView: View {
    var model: MainViewModel

    var body: some View {
        VStack(spacing: 20) {
            Button(action: { model.pushRoute(Route.newWallet(route: NewWalletRoute.select)) }) {
                Text("Push Route")
            }

            Button(action: { model.setRoute([Route.newWallet(route: NewWalletRoute.select)]) }) {
                Text("Set Route")
            }

            Button(action: { try! model.database.toggleBoolConfig(key: GlobalBoolConfigKey.completedOnboarding) }) {
                Text("Onboarding: \(try! model.database.getBoolConfig(key: GlobalBoolConfigKey.completedOnboarding))")
            }
        }
        .padding()
    }
}

#Preview {
    CoveView(model: MainViewModel())
}
