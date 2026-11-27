@testable import Cove
import CoveCore
import Darwin
import XCTest

/// The interfaces the node search sweeps must be the device's own Wi-Fi or Ethernet networks, each
/// carrying the index of the interface its sockets are bound to
///
/// The simulator reports this Mac's interfaces, which include a Wi-Fi or Ethernet `en*` one on any
/// machine that runs the tests
final class LanNodeDiscoveryTests: XCTestCase {
    func testLanInterfacesAreTheDevicesPrivateIpv4Networks() {
        let interfaces = LanNodeDiscovery.lanInterfaces()

        XCTAssertFalse(interfaces.isEmpty, "this machine is on a Wi-Fi or Ethernet network")
        for lan in interfaces {
            var address = in_addr()
            XCTAssertEqual(inet_pton(AF_INET, lan.ipv4Address, &address), 1, "\(lan) is IPv4")
            XCTAssertTrue((1 ... 32).contains(lan.prefixLength), "\(lan) has a prefix length")
            XCTAssertNotEqual(lan.ipv4Address, "127.0.0.1", "\(lan) is loopback")

            guard case let .interfaceIndex(index) = lan.binding else {
                return XCTFail("\(lan) is not bound to an interface index")
            }

            var name = [CChar](repeating: 0, count: Int(IF_NAMESIZE))
            XCTAssertNotNil(if_indextoname(index, &name), "\(lan) names no interface")
            XCTAssertTrue(String(cString: name).hasPrefix("en"), "\(lan) is not a Wi-Fi or Ethernet interface")
        }
    }
}
