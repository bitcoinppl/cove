@testable import CoveCore
import Testing

@Test func cvcAcceptsAsciiDigitsWithinTheProtocolRange() {
    #expect(tapSignerCvcInputError(value: "123456") == nil)
    #expect(tapSignerCvcInputError(value: String(repeating: "7", count: 32)) == nil)
}

@Test func cvcRejectsValuesOutsideTheProtocolRange() {
    #expect(tapSignerCvcInputError(value: "12345") == .invalidLength)
    #expect(tapSignerCvcInputError(value: String(repeating: "1", count: 33)) == .invalidLength)
}

@Test func cvcRejectsNonAsciiDigitsWithUserFacingError() {
    #expect(tapSignerCvcInputError(value: "12345a") == .invalidCharacters)
    #expect(tapSignerCvcInputError(value: "１２３４５６") == .invalidCharacters)
    #expect(TapSignerCvcInputError.invalidCharacters.errorDescription == "Enter the CVC as ASCII digits only.")
    #expect(TapSignerCvcInputError.invalidLength.errorDescription == "Enter between 6 and 32 ASCII digits.")
}

@Test func chainCodeRequiresExactly32DecodedBytes() {
    let valid = String(repeating: "ab", count: 32)

    #expect(tapSignerChainCodeBytes(hex: valid)?.count == 32)
    #expect(tapSignerChainCodeBytes(hex: String(repeating: "ab", count: 31)) == nil)
    #expect(tapSignerChainCodeBytes(hex: String(repeating: "ab", count: 33)) == nil)
    #expect(tapSignerChainCodeBytes(hex: String(repeating: "gg", count: 32)) == nil)
    #expect(tapSignerChainCodeInputError(hex: String(repeating: "gg", count: 32)) == .invalidHex)
    #expect(TapSignerChainCodeInputError.invalidHex.errorDescription == "Enter hexadecimal characters only.")
    #expect(TapSignerChainCodeInputError.invalidLength.errorDescription == "Enter exactly 64 hexadecimal characters (32 bytes).")
}
