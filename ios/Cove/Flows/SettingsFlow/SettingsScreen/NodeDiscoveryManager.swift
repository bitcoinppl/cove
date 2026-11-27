//
//  NodeDiscoveryManager.swift
//  Cove
//

import Foundation

extension WeakReconciler: NodeDiscoveryManagerReconciler where Reconciler == NodeDiscoveryManager {}

/// Runs node settings' searches of the local network for Electrum servers
///
/// The screen owns one while it is shown. Rust runs each search and lists every server as its
/// check passes; this hands it the Wi-Fi and Ethernet interfaces to search. Releasing the manager
/// cancels a running search, while the screen only disappearing, as it does under the privacy
/// cover, leaves the search running
@Observable final class NodeDiscoveryManager: ReconcilingManager, NodeDiscoveryManagerReconciler {
    typealias Message = NodeDiscoveryReconcileMessage

    @ObservationIgnored private let rust: RustNodeDiscoveryManager
    private(set) var state: NodeDiscoveryState

    init() {
        let rust = RustNodeDiscoveryManager()
        self.rust = rust
        state = rust.state()
        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    var isSearching: Bool {
        if case .searching = state { true } else { false }
    }

    /// Searches the Wi-Fi and Ethernet networks the phone is on, unless a search is running
    func start() {
        guard !isSearching else { return }
        rust.start(interfaces: LanNodeDiscovery.lanInterfaces())
    }

    /// Stops the running search, keeping the servers it found
    func stop() {
        rust.stop()
    }

    func apply(_ message: Message) {
        switch message {
        case .started:
            state = .searching(found: [])

        case let .found(index, server):
            guard case var .searching(found) = state else { return }
            found.insert(server, at: min(Int(index), found.count))
            state = .searching(found: found)

        case .stopped:
            state = .stopped(found: state.foundServers)

        case .finished:
            state = .finished(found: state.foundServers)

        case let .failed(error):
            state = .failed(error: error)
        }
    }
}

extension NodeDiscoveryState {
    /// The servers found so far, none before a search or when it failed
    var foundServers: [DiscoveredNode] {
        switch self {
        case .idle, .failed: []
        case let .searching(found), let .stopped(found), let .finished(found): found
        }
    }
}
