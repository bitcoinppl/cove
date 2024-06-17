//
//  CoveApp.swift
//  Cove
//
//  Created by Justin  on 6/4/24.
//

import SwiftUI

@main
struct CoveApp: App {
    @State var rust: MainViewModel

    public init() {
        self.rust = MainViewModel()
    }

    var body: some Scene {
        WindowGroup {
            HStack {
                Button(action: {
                    self.rust.dispatch(event: .setRoute(route: Route.cove))
                }) {
                    Text("Cove")
                }
            }
            Text(String(describing: self.rust.router.route))

            switch rust.router.route {
            case .cove:
                Cove(model: self.rust)
            }
        }
    }
}
