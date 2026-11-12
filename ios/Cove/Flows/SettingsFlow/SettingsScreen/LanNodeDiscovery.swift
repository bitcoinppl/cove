//
//  LanNodeDiscovery.swift
//  Cove
//

import Darwin
import Foundation

/// Finds Electrum servers on the Wi-Fi or Ethernet network the phone is on
///
/// Rust owns the search and keeps it on the local network; this supplies the interfaces it may
/// search, with the index Rust binds every socket to so a VPN can't carry the search. iOS asks
/// for Local Network access the first time the search connects, and can refuse those first
/// connections before the person answers, so a first search can come back empty
enum LanNodeDiscovery {
    static func discover(nodeSelector: NodeSelector) async throws -> [DiscoveredNode] {
        try await nodeSelector.discoverLanNodes(interfaces: lanInterfaces())
    }

    /// The IPv4 address and prefix of each Wi-Fi or wired Ethernet interface that is up
    ///
    /// iOS names those `en*`, while cellular is `pdp_ip*` and VPNs are `utun*` or `ipsec*`, so
    /// neither is ever searched. Rust also drops any address that isn't private
    static func lanInterfaces() -> [LanInterface] {
        var head: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&head) == 0, let first = head else { return [] }
        defer { freeifaddrs(head) }

        return sequence(first: first, next: { $0.pointee.ifa_next })
            .compactMap { lanInterface($0.pointee) }
    }

    private static func lanInterface(_ entry: ifaddrs) -> LanInterface? {
        let flags = Int32(entry.ifa_flags)
        let isUp = flags & IFF_UP != 0 && flags & IFF_RUNNING != 0 && flags & IFF_LOOPBACK == 0

        guard isUp,
              String(cString: entry.ifa_name).hasPrefix("en"),
              let address = entry.ifa_addr,
              address.pointee.sa_family == UInt8(AF_INET),
              let netmask = entry.ifa_netmask
        else { return nil }

        let index = if_nametoindex(entry.ifa_name)
        guard index != 0 else { return nil }

        var ip = address.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { $0.pointee.sin_addr }
        let mask = netmask.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { $0.pointee.sin_addr.s_addr }

        var text = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
        guard inet_ntop(AF_INET, &ip, &text, socklen_t(INET_ADDRSTRLEN)) != nil else { return nil }
        let ipv4Address = text.withUnsafeBufferPointer { String(cString: $0.baseAddress!) }

        // a netmask's set bits are its prefix length in either byte order
        return LanInterface(
            ipv4Address: ipv4Address,
            prefixLength: UInt8(mask.nonzeroBitCount),
            binding: .interfaceIndex(index: index)
        )
    }
}
