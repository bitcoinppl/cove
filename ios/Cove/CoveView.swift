import SwiftUI

struct Cove: View {
    @State var model: MainViewModel

    var body: some View {
        HStack {
            Text("This is my first time launching? \(try! model.database.getBoolConfig(key: .completedOnboarding))")
            Button(action: {
                try! model.database.toggleBoolConfig(key: .completedOnboarding)
            }) {
                Text("Toggle")
            }
        }
        .padding()
    }
}
