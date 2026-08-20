@testable import CoveCore
import Testing

@Test func cvcValidationDelegatesToRustRules() {
    #expect(tapSignerCvcInputError(value: "123456") == nil)
    #expect(tapSignerCvcInputError(value: String(repeating: "7", count: 32)) == nil)
    #expect(tapSignerCvcInputError(value: "12345") != nil)
    #expect(tapSignerCvcInputError(value: String(repeating: "1", count: 33)) != nil)
    #expect(tapSignerCvcInputError(value: "12345a") != nil)
    #expect(tapSignerCvcInputError(value: "１２３４５６") != nil)
}

@Test func chainCodeRequiresExactly32DecodedBytes() {
    let valid = String(repeating: "ab", count: 32)

    #expect(tapSignerChainCodeBytes(hex: valid)?.count == 32)
    #expect(tapSignerChainCodeBytes(hex: String(repeating: "ab", count: 31)) == nil)
    #expect(tapSignerChainCodeBytes(hex: String(repeating: "ab", count: 33)) == nil)
    #expect(tapSignerChainCodeBytes(hex: String(repeating: "gg", count: 32)) == nil)
    #expect(tapSignerChainCodeInputError(hex: String(repeating: "gg", count: 32)) != nil)
}
