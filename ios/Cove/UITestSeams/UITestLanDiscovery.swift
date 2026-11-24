//
//  UITestLanDiscovery.swift
//  Cove
//
//  Debug-only stand-in for the local network node search in UI tests
//

#if DEBUG
    import Foundation

    /// Search results a UI test feeds node settings instead of searching the local network
    ///
    /// A UI test can't put an Electrum server on the network the simulator is on, and every
    /// simulator on a machine shares that network, so the search itself is left to the Rust tests.
    /// When `COVE_UI_TEST_LAN_DISCOVERY` holds `{"searches": [...]}`, each search the screen
    /// starts takes the next entry instead, and the last entry answers every search after it.
    /// An entry fails with its `error`, `noLocalNetwork` or `localNetworkBlocked`, or finds its
    /// `servers`, each a `host` and an Electrum `port` on this machine's loopback, which the
    /// simulator shares, saved for the selected network
    enum UITestLanDiscovery {
        static let environmentKey = "COVE_UI_TEST_LAN_DISCOVERY"

        private struct Script: Decodable {
            let searches: [Search]
        }

        private struct Search: Decodable {
            let error: SearchError?
            let servers: [Server]?
        }

        private enum SearchError: String, Decodable {
            case noLocalNetwork
            case localNetworkBlocked
        }

        private struct Server: Decodable {
            let host: String
            let port: UInt16
        }

        @MainActor private static var searchesMade = 0

        /// The scripted search's result, or nil when no test set the variable
        @MainActor
        static func discover() async throws -> [DiscoveredNode]? {
            guard let json = ProcessInfo.processInfo.environment[environmentKey] else { return nil }

            let script: Script
            do {
                script = try JSONDecoder().decode(Script.self, from: Data(json.utf8))
            } catch {
                fatalError("\(environmentKey) is not a valid search script: \(error)")
            }

            guard let last = script.searches.last else { fatalError("\(environmentKey) has no searches") }
            let search = searchesMade < script.searches.count ? script.searches[searchesMade] : last
            searchesMade += 1

            switch search.error {
            case .noLocalNetwork: throw NodeDiscoveryError.NoLocalNetwork
            case .localNetworkBlocked: throw NodeDiscoveryError.LocalNetworkBlocked
            case nil: break
            }

            let network = Database().globalConfig().selectedNetwork()
            return (search.servers ?? []).map { server in
                let address = "127.0.0.1:\(server.port)"
                let node = Node(name: server.host, network: network, apiType: .electrum, url: "tcp://\(address)")
                return DiscoveredNode(node: node, host: server.host, address: address, software: nil)
            }
        }
    }
#endif
