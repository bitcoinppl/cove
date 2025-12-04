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

public extension SetupCmdResponse {
    var error: TapSignerReaderError? {
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

public extension AfterPinAction {
    var userMessage: String {
        afterPinActionUserMessage(action: self)
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

public extension HardwareWalletMetadata {
    func isTapSigner() -> Bool {
        hardwareWalletIsTapSigner(hardwareWallet: self)
    }
}

extension SendFlowAlertState {
    init(_ addressError: AddressError, address: String) {
        self = addressErrorToAlertState(error: addressError, address: address)
    }
}

extension Utxo: @retroactive Identifiable {
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
        coinControlListSortKeyToString(key: self)
    }
}
