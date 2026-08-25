@testable import Cove
import Security
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

    func testKeychainEnumerationAcceptsOnlySuccessAndItemNotFound() {
        let items = [[kSecAttrAccount as String: "wallet::wallet_mnemonic"]] as CFArray

        guard case let .success(accounts) = classifyKeychainAccountEnumeration(
            status: errSecSuccess,
            result: items
        ) else {
            return XCTFail("success status must return the enumerated accounts")
        }
        XCTAssertEqual(accounts, ["wallet::wallet_mnemonic"])

        guard case .itemNotFound = classifyKeychainAccountEnumeration(
            status: errSecItemNotFound,
            result: nil
        ) else {
            return XCTFail("item-not-found must be the only empty success")
        }

        guard case let .failure(status) = classifyKeychainAccountEnumeration(
            status: errSecInteractionNotAllowed,
            result: nil
        ) else {
            return XCTFail("keychain access errors must fail closed")
        }
        XCTAssertEqual(status, errSecInteractionNotAllowed)
    }

    func testWalletKeyDeletionRejectsEnumerationFailure() {
        let accessor = KeychainAccessor {
            .failure(errSecInteractionNotAllowed)
        }

        XCTAssertThrowsError(try accessor.deleteAllWalletItems()) { error in
            XCTAssertEqual(error as? KeychainError, .Delete)
        }
    }
}
