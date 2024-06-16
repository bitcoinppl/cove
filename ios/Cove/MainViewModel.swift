import SwiftUI

@Observable class ViewModel: FfiUpdater {
    var rust: FfiApp;
    var count: Int32;
    var timer: TimerState;
    var router: Router;
    
    public init() {
        let rust = FfiApp()
        let state = rust.getState()

        self.count = state.count
        self.timer = state.timer
        self.router = state.router
        self.rust = rust

        self.rust.listenForUpdates(updater: self)
    }
    
    func update(update: Update) {
        print("update: $update")
        switch update {
        case .countChanged(count: let count):
            self.count = count
        case .timer(state: let timer):
            self.timer = timer
        case .router(router: let router):
            self.router = router
        }
        print(self.router)
        print(self.count)
        print(self.timer)
        print()
    }
    
    public func dispatch(event: Event) {
        self.rust.dispatch(event: event)
    }
}
