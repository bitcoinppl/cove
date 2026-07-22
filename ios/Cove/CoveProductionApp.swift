import SwiftUI

@main
struct CoveApp: App {
    @UIApplicationDelegateAdaptor(CoveAppDelegate.self) private var appDelegate

    private let root = CoveApplicationRoot(dependencies: .production())

    var body: some Scene {
        WindowGroup {
            root
        }
    }
}
