@testable import Cove
import XCTest

final class KeyTeleportSecurityHelpersTests: XCTestCase {
    func testPacketRoutingAcceptsAnUnownedFlow() {
        XCTAssertEqual(
            KeyTeleportPacketRoutingDecision.resolve(
                activeDirections: [],
                requiredDirection: .send
            ),
            .accept
        )
    }

    func testPacketRoutingAcceptsTheActiveDirection() {
        XCTAssertEqual(
            KeyTeleportPacketRoutingDecision.resolve(
                activeDirections: [.receive, .receive],
                requiredDirection: .receive
            ),
            .accept
        )
    }

    func testPacketRoutingRejectsTheOppositeDirection() {
        XCTAssertEqual(
            KeyTeleportPacketRoutingDecision.resolve(
                activeDirections: [.receive],
                requiredDirection: .send
            ),
            .rejectConflict
        )
    }

    func testPacketRoutingRejectsAnInconsistentRouteStack() {
        XCTAssertEqual(
            KeyTeleportPacketRoutingDecision.resolve(
                activeDirections: [.receive, .send],
                requiredDirection: .send
            ),
            .rejectConflict
        )
    }

    func testSendStartAcceptsAnUnownedFlow() {
        XCTAssertEqual(
            KeyTeleportSendStartDecision.resolve(activeDirections: []),
            .start
        )
    }

    func testSendStartRejectsAnyActiveFlowDirection() {
        XCTAssertEqual(
            KeyTeleportSendStartDecision.resolve(activeDirections: [.receive]),
            .rejectActiveFlow
        )
        XCTAssertEqual(
            KeyTeleportSendStartDecision.resolve(activeDirections: [.send]),
            .rejectActiveFlow
        )
    }
}
