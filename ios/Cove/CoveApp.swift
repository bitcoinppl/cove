//
//  CoveApp.swift
//  Cove
//
//  Created by Justin  on 6/4/24.
//

import SwiftUI

@main
struct CoveApp: App {
    @State var rust: ViewModel;
    
    public init() {
        self.rust = ViewModel()
    }

    var body: some Scene {

        WindowGroup {
            HStack {
                Button(action: {
                    self.rust.dispatch(event: .setRoute(route: Route.cove))
                }) {
                    Text("Cove")
                }
                Button(action: {
                    self.rust.dispatch(event: .setRoute(route: Route.timer))
                }) {
                    Text("Timer")
                }
            }
            Text(String(describing: self.rust.router.route))

            switch rust.router.route {
            case .cove:
                Cove(rust: self.rust)
            case .timer:
                Timer(rust: self.rust)
            }
        }
    }
}
