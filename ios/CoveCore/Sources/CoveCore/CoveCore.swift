import Foundation
@_exported import cove_core_ffi

extension WalletAddressType: @retroactive Comparable {
    public static func < (lhs: WalletAddressType, rhs: WalletAddressType) -> Bool {
        walletAddressTypeLessThan(lhs: lhs, rhs: rhs)
    }
}

extension DiscoveryState: @retroactive Equatable {
    public static func == (lhs: DiscoveryState, rhs: DiscoveryState) -> Bool {
        discoveryStateIsEqual(lhs: lhs, rhs: rhs)
    }
}

extension FeeSpeed {
    public var string: String {
        feeSpeedToString(feeSpeed: self)
    }

    public var duration: String {
        feeSpeedDuration(feeSpeed: self)
    }

    public var isCustom: Bool {
        feeSpeedIsCustom(feeSpeed: self)
    }
}

extension Amount: Equatable {
    public static func == (lhs: Amount, rhs: Amount) -> Bool {
        lhs.asSats() == rhs.asSats()
    }
}

extension PriceResponse: Equatable {
    public static func == (lhs: PriceResponse, rhs: PriceResponse) -> Bool {
        pricesAreEqual(lhs: lhs, rhs: rhs)
    }
}

extension SendRoute {
    public func id() -> WalletId {
        switch self {
        case let .setAmount(id, address: _, amount: _): id
        case let .confirm(args): args.id
        case let .hardwareExport(id: id, details: _): id
        }
    }
}

extension UnsignedTransaction: Identifiable {
    public var ID: TxId {
        id()
    }
}

extension [BoxedRoute] {
    public var routes: [Route] {
        map { $0.route() }
    }
}

extension FeeRateOptionsWithTotalFee: Equatable {
    public static func == (lhs: FeeRateOptionsWithTotalFee, rhs: FeeRateOptionsWithTotalFee) -> Bool
    {
        feeRateOptionsWithTotalFeeIsEqual(lhs: lhs, rhs: rhs)
    }
}

extension FeeRateOptionWithTotalFee: Equatable {
    public static func == (lhs: FeeRateOptionWithTotalFee, rhs: FeeRateOptionWithTotalFee) -> Bool {
        lhs.isEqual(rhs: rhs)
    }
}

extension FiatOrBtc {
    public func toggle() -> FiatOrBtc {
        self == .fiat ? .btc : .fiat
    }
}

extension LabelManager {
    public func `import`(labels: Bip329Labels) throws {
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

extension Data {
    public func hexEncodedString() -> String {
        map { String(format: "%02hhx", $0) }.joined()
    }
}

extension SetupCmdResponse {
    public var error: TapSignerReaderError? {
        switch self {
        case .complete: .none
        case let .continueFromInit(continueCmd): continueCmd.error
        case let .continueFromBackup(continueCmd): continueCmd.error
        case let .continueFromDerive(continueCmd): continueCmd.error
        }
    }
}

extension TapSignerRoute: Equatable, Hashable {
    public static func == (lhs: TapSignerRoute, rhs: TapSignerRoute) -> Bool {
        isTapSignerRouteEqual(lhs: lhs, rhs: rhs)
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(self)
    }
}

extension TapSignerResponse {
    public var setupResponse: SetupCmdResponse? {
        tapSignerResponseSetupResponse(response: self)
    }

    public var deriveResponse: DeriveInfo? {
        tapSignerResponseDeriveResponse(response: self)
    }

    public var backupResponse: Data? {
        tapSignerResponseBackupResponse(response: self)
    }

    public var signResponse: Psbt? {
        tapSignerResponseSignResponse(response: self)
    }

    public var isChangeResponse: Bool {
        tapSignerResponseChangeResponse(response: self)
    }
}

extension AfterPinAction {
    public var userMessage: String {
        afterPinActionUserMessage(action: self)
    }
}

extension TapSignerConfirmPinArgs {
    public init(from: TapSignerNewPinArgs, newPin: String) {
        self = tapSignerConfirmPinArgsNewFromNewPin(args: from, newPin: newPin)
    }
}

extension TapSigner: @retroactive Equatable {
    public static func == (lhs: TapSigner, rhs: TapSigner) -> Bool {
        lhs.isEqual(rhs: rhs)
    }
}

extension WalletMetadata {
    public func isTapSigner() -> Bool {
        hardwareMetadata?.isTapSigner() ?? false
    }

    public func identOrFingerprint() -> String {
        if case let .tapSigner(t) = hardwareMetadata {
            return t.fullCardIdent()
        }

        return masterFingerprint?.asUppercase() ?? "No Fingerprint"
    }
}

extension HardwareWalletMetadata {
    public func isTapSigner() -> Bool {
        hardwareWalletIsTapSigner(hardwareWallet: self)
    }
}

extension SendFlowAlertState {
    init(_ addressError: AddressError, address: String) {
        self = addressErrorToAlertState(error: addressError, address: address)
    }
}

extension Utxo: @retroactive Identifiable, @retroactive Hashable, @retroactive Equatable {
    public typealias ID = OutPoint

    public var id: OutPoint {
        self.outpoint
    }

    public var name: String {
        utxoName(utxo: self)
    }

    public var date: String {
        utxoDate(utxo: self)
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(utxoHashToUint(utxo: self))
    }

    public static func == (lhs: Self, rhs: Self) -> Bool {
        utxoIsEqual(lhs: lhs, rhs: rhs)
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

extension CoinControlListSortKey {
    public var title: String {
        coinControlListSortKeyToString(key: self)
    }
}
