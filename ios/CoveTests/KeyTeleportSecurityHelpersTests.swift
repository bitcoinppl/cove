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

    func testKeychainItemReadTreatsOnlyItemNotFoundAsAbsent() {
        let data = Data("stored".utf8) as CFData

        guard case let .value(value) = classifyKeychainItemRead(status: errSecSuccess, result: data)
        else {
            return XCTFail("success status must return the stored value")
        }
        XCTAssertEqual(value, "stored")

        guard case .itemNotFound = classifyKeychainItemRead(status: errSecItemNotFound, result: nil)
        else {
            return XCTFail("item-not-found must be the only absent result")
        }

        guard case let .failure(status) = classifyKeychainItemRead(
            status: errSecInteractionNotAllowed,
            result: nil
        ) else {
            return XCTFail("a locked keychain must fail closed")
        }
        XCTAssertEqual(status, errSecInteractionNotAllowed)

        guard case .failure(errSecDecode) = classifyKeychainItemRead(
            status: errSecSuccess,
            result: Data([0xFF]) as CFData
        ) else {
            return XCTFail("undecodable data must fail closed")
        }
    }

    func testKeychainGetThrowsOnReadFailureAndReturnsNilOnlyWhenMissing() throws {
        let locked = KeychainAccessor(itemReader: { _ in .failure(errSecInteractionNotAllowed) })
        XCTAssertThrowsError(try locked.get(key: "cspp::v1::master_key")) { error in
            guard case .Read = error as? KeychainError else {
                return XCTFail("expected KeychainError.Read, got \(error)")
            }
        }

        let missing = KeychainAccessor(itemReader: { _ in .itemNotFound })
        XCTAssertNil(try missing.get(key: "cspp::v1::master_key"))
    }

    func testWalletKeyDeletionRejectsEnumerationFailure() {
        let accessor = KeychainAccessor(accountEnumerator: {
            .failure(errSecInteractionNotAllowed)
        })

        XCTAssertThrowsError(try accessor.deleteAllWalletItems()) { error in
            XCTAssertEqual(error as? KeychainError, .Delete)
        }
    }
}
