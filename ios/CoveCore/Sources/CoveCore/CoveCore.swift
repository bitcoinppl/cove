@_exported import cove_core_ffi
import Foundation

public extension FeeSpeed {
    var string: String {
        self.description
    }

    var duration: String {
        feeSpeedDuration(feeSpeed: self)
    }

    var isCustom: Bool {
        feeSpeedIsCustom(feeSpeed: self)
    }
}

extension Amount: Equatable {
    public static func == (lhs: Amount, rhs: Amount) -> Bool {
        lhs.asSats() == rhs.asSats()
    }
}

extension AppAlertState: Equatable {
    public static func == (lhs: AppAlertState, rhs: AppAlertState) -> Bool {
        lhs.isEqual(rhs: rhs)
    }
}

extension PriceResponse: Equatable {
    public static func == (lhs: PriceResponse, rhs: PriceResponse) -> Bool {
        pricesAreEqual(lhs: lhs, rhs: rhs)
    }
}

public extension SendRoute {
    func id() -> WalletId {
        switch self {
        case let .setAmount(id, address: _, amount: _): id
        case let .coinControlSetAmount(id: id, utxos: _): id
        case let .confirm(args): args.id
        case let .hardwareExport(id: id, details: _): id
        }
    }
}

public extension CoinControlRoute {
    func id() -> WalletId {
        switch self {
        case let .list(id): id
        }
    }
}

extension UnsignedTransaction: Identifiable {
    public var ID: TxId {
        id()
    }
}

public extension [BoxedRoute] {
    var routes: [Route] {
        map { $0.route() }
    }
}

extension FeeRateOptionsWithTotalFee: Equatable {
    public static func == (lhs: FeeRateOptionsWithTotalFee, rhs: FeeRateOptionsWithTotalFee) -> Bool {
        feeRateOptionsWithTotalFeeIsEqual(lhs: lhs, rhs: rhs)
    }
}

extension FeeRateOptionWithTotalFee: Equatable {
    public static func == (lhs: FeeRateOptionWithTotalFee, rhs: FeeRateOptionWithTotalFee) -> Bool {
        lhs.isEqual(rhs: rhs)
    }
}

public extension FiatOrBtc {
    func toggle() -> FiatOrBtc {
        self == .fiat ? .btc : .fiat
    }
}

public extension LabelManager {
    func `import`(labels: Bip329Labels) throws {
        try importLabels(labels: labels)
    }
}

extension NfcMessage? {
    init(_ string: String?, _ data: Data?) {
        self = try? NfcMessage.tryNew(string: string, data: data)
    }

    init(string: String?, data: Data?) {
        self = try? NfcMessage.tryNew(string: string, data: data)
    }

    init(string: String, data: Data? = nil) {
        self = try? NfcMessage.tryNew(string: string, data: data)
    }
}

extension NfcMessage: Equatable {
    public func tryIntoMultiFormat() throws -> MultiFormat {
        try multiFormatTryFromNfcMessage(nfcMessage: self)
    }

    public static func == (lhs: NfcMessage, rhs: NfcMessage) -> Bool {
        nfcMessageIsEqual(lhs: lhs, rhs: rhs)
    }
}

public extension Data {
    func hexEncodedString() -> String {
        map { String(format: "%02hhx", $0) }.joined()
    }
}

/// Errors raised when a TAPSIGNER CVC input is not six to 32 ASCII digits
public enum TapSignerCvcInputError: Error, Equatable, LocalizedError {
    case invalidCharacters
    case invalidLength

    public var errorDescription: String? {
        switch self {
        case .invalidCharacters:
            "Enter the CVC as ASCII digits only."
        case .invalidLength:
            "Enter between 6 and 32 ASCII digits."
        }
    }
}

/// Errors raised when a TAPSIGNER chain-code input is not exactly 32 bytes
public enum TapSignerChainCodeInputError: Error, Equatable, LocalizedError {
    case invalidHex
    case invalidLength

    public var errorDescription: String? {
        switch self {
        case .invalidHex:
            "Enter hexadecimal characters only."
        case .invalidLength:
            "Enter exactly 64 hexadecimal characters (32 bytes)."
        }
    }
}

private func decodeStrictHex(_ value: String, minBytes: Int, maxBytes: Int) -> Data? {
    let bytes = Array(value.utf8)
    guard bytes.count == value.count,
          bytes.count.isMultiple(of: 2),
          (minBytes ... maxBytes).contains(bytes.count / 2)
    else { return nil }

    var decoded = Data(capacity: bytes.count / 2)

    for index in stride(from: 0, to: bytes.count, by: 2) {
        guard let high = hexNibble(bytes[index]),
              let low = hexNibble(bytes[index + 1])
        else { return nil }

        decoded.append((high << 4) | low)
    }

    return decoded
}

private func hexNibble(_ value: UInt8) -> UInt8? {
    switch value {
    case 48 ... 57: value - 48
    case 65 ... 70: value - 55
    case 97 ... 102: value - 87
    default: nil
    }
}

/// Return the user-facing validation error for an invalid TAPSIGNER CVC
public func tapSignerCvcInputError(value: String) -> TapSignerCvcInputError? {
    let bytes = Array(value.utf8)
    guard bytes.allSatisfy({ (48 ... 57).contains($0) }) else {
        return .invalidCharacters
    }

    guard (6 ... 32).contains(bytes.count) else { return .invalidLength }

    return nil
}

/// Build an opaque TAPSIGNER CVC after enforcing its six-to-32 ASCII digit format
public func makeTapSignerCvc(value: String) throws -> TapSignerCvc {
    if let inputError = tapSignerCvcInputError(value: value) {
        throw inputError
    }

    return try TapSignerCvc.tryNew(value: value)
}

/// Decode a TAPSIGNER chain code only when it is exactly 32 bytes of hexadecimal data
public func tapSignerChainCodeBytes(hex: String) -> Data? {
    decodeStrictHex(hex, minBytes: 32, maxBytes: 32)
}

/// Return the user-facing validation error for an invalid TAPSIGNER chain code
public func tapSignerChainCodeInputError(hex: String) -> TapSignerChainCodeInputError? {
    guard tapSignerChainCodeBytes(hex: hex) == nil else { return nil }

    let length = hex.utf8.count
    guard length == 64 else { return .invalidLength }

    return .invalidHex
}

/// Build an exact 32-byte TAPSIGNER chain code or return a user-facing validation error
public func makeTapSignerChainCode(hex: String) throws -> Data {
    guard let bytes = tapSignerChainCodeBytes(hex: hex) else {
        throw tapSignerChainCodeInputError(hex: hex) ?? .invalidHex
    }

    return bytes
}

public extension SetupCmdResponse {
    var error: TapSignerReaderError? {
        switch self {
        case .complete: .none
        case let .retry(continuation): continuation.error()
        }
    }
}

extension TapSignerRoute: Equatable, Hashable {
    public static func == (lhs: TapSignerRoute, rhs: TapSignerRoute) -> Bool {
        lhs.isEqual(other: rhs)
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(self)
    }
}

public extension TapSignerResponse {
    var setupResponse: SetupCmdResponse? {
        tapSignerResponseSetupResponse(response: self)
    }

    var deriveResponse: DeriveInfo? {
        tapSignerResponseDeriveResponse(response: self)
    }

    var backupResponse: Data? {
        tapSignerResponseBackupResponse(response: self)
    }

    var signResponse: Psbt? {
        tapSignerResponseSignResponse(response: self)
    }

    var isChangeResponse: Bool {
        tapSignerResponseChangeResponse(response: self)
    }
}

extension AfterPinAction: @retroactive Equatable {
    public static func == (lhs: AfterPinAction, rhs: AfterPinAction) -> Bool {
        switch (lhs, rhs) {
        case (.derive, .derive): true
        case (.change, .change): true
        case (.backup, .backup): true
        case let (.sign(lhsPsbt), .sign(rhsPsbt)): lhsPsbt.txId() == rhsPsbt.txId()
        default: false
        }
    }
}

public extension TapSignerConfirmPinArgs {
    init(from: TapSignerNewPinArgs, newPin: String) {
        self = tapSignerConfirmPinArgsNewFromNewPin(args: from, newPin: newPin)
    }
}

extension TapSigner: @retroactive Equatable {
    public static func == (lhs: TapSigner, rhs: TapSigner) -> Bool {
        lhs.isEqual(rhs: rhs)
    }
}

extension QrDensity: @retroactive Equatable {
    public static func == (lhs: QrDensity, rhs: QrDensity) -> Bool {
        qrDensityIsEqual(lhs: lhs, rhs: rhs)
    }
}

public extension WalletMetadata {
    func isTapSigner() -> Bool {
        hardwareMetadata?.isTapSigner() ?? false
    }

    func identOrFingerprint() -> String {
        if case let .tapSigner(t) = hardwareMetadata {
            return t.fullCardIdent()
        }

        return masterFingerprint?.asUppercase() ?? "No Fingerprint"
    }
}

extension SendFlowAlertState {
    init(_ addressError: AddressError, address: String) {
        self = sendFlowAlertStateFromAddressError(error: addressError, address: address)
    }
}

extension Utxo: @retroactive Identifiable {
    public typealias ID = OutPoint

    public var id: OutPoint {
        self.outpoint
    }
}

extension OutPoint: @retroactive Hashable, Equatable {
    public func hash(into hasher: inout Hasher) {
        hasher.combine(self.hashToUint())
    }

    public static func == (lhs: OutPoint, rhs: OutPoint) -> Bool {
        lhs.eq(rhs: rhs)
    }
}

public extension CoinControlListSortKey {
    var title: String {
        description
    }
}
