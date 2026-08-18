@testable import CoveCore
import Foundation
import Testing

@Test func factoryCodeUsesHexEncodedAsciiBytes() {
    #expect(
        tapSignerCvcBytes(hex: "313233343536") == Data([0x31, 0x32, 0x33, 0x34, 0x35, 0x36])
    )
}

@Test func cvcComparisonUsesDecodedBytes() {
    #expect(
        tapSignerCvcBytes(hex: "ABCDEF010203") == tapSignerCvcBytes(hex: "abcdef010203")
    )
}

@Test func cvcRejectsValuesOutsideTheProtocolRange() {
    #expect(tapSignerCvcBytes(hex: "3132333435") == nil)
    #expect(tapSignerCvcBytes(hex: String(repeating: "31", count: 33)) == nil)
    #expect(tapSignerCvcBytes(hex: "31323334353z") == nil)
    #expect(tapSignerCvcInputError(hex: String(repeating: "31", count: 33)) == .invalidLength)
    #expect(tapSignerCvcInputError(hex: "31323334353z") == .invalidHex)
}

@Test func chainCodeRequiresExactly32DecodedBytes() {
    let valid = String(repeating: "ab", count: 32)

    #expect(tapSignerChainCodeBytes(hex: valid)?.count == 32)
    #expect(tapSignerChainCodeBytes(hex: String(repeating: "ab", count: 31)) == nil)
    #expect(tapSignerChainCodeBytes(hex: String(repeating: "ab", count: 33)) == nil)
}
