import SwiftUI

@Observable class MainViewModel: FfiUpdater {
    var rust: FfiApp
    var router: Router
    var database: Database

    public init() {
        let rust = FfiApp()
        let state = rust.getState()

        self.router = state.router
        self.rust = rust
        self.database = Database()

        self.rust.listenForUpdates(updater: self)
    }

    func update(update: Update) {
        print("[swift] update: \(update)")
        switch update {
        case .routerUpdate(router: let router):
            self.router = router
        case .databaseUpdate:
            self.database = Database()
        }
    }

    public func dispatch(event: Event) {
        self.rust.dispatch(event: event)
    }
}
