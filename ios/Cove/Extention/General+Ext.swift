//
//  General+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 10/20/24.
//

// General extention for types from rust
import Foundation
import SwiftUI

extension WalletAddressType: Comparable {
    public static func < (lhs: WalletAddressType, rhs: WalletAddressType) -> Bool {
        walletAddressTypeLessThan(lhs: lhs, rhs: rhs)
    }
}

extension DiscoveryState: Equatable {
    public static func == (lhs: DiscoveryState, rhs: DiscoveryState) -> Bool {
        discoveryStateIsEqual(lhs: lhs, rhs: rhs)
    }
}

extension Address: Equatable {
    public static func == (lhs: Address, rhs: Address) -> Bool {
        addressIsEqual(lhs: lhs, rhs: rhs)
    }
}

extension FeeSpeed {
    var string: String {
        feeSpeedToString(feeSpeed: self)
    }

    var duration: String {
        feeSpeedDuration(feeSpeed: self)
    }

    var circleColor: Color {
        Color(feeSpeedToCircleColor(feeSpeed: self))
    }

    var isCustom: Bool {
        feeSpeedIsCustom(feeSpeed: self)
    }
}

extension Double {
    func btcFmt(maxDecimals: Int = 10) -> String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.minimumFractionDigits = maxDecimals
        formatter.maximumFractionDigits = maxDecimals
        formatter.usesGroupingSeparator = false
        return formatter.string(from: NSNumber(value: self))!
    }

    func btcFmtWithUnit() -> String {
        btcFmt() + " BTC"
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
        case let .confirm(id: id, details: _, signedTransaction: _): id
        case let .hardwareExport(id: id, details: _): id
        }
    }
}

extension UnsignedTransaction: Identifiable {
    var ID: TxId {
        id()
    }
}

extension [BoxedRoute] {
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

extension FiatOrBtc {
    func toggle() -> FiatOrBtc {
        self == .fiat ? .btc : .fiat
    }
}

extension LabelManager {
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
    public static func == (lhs: NfcMessage, rhs: NfcMessage) -> Bool {
        nfcMessageIsEqual(lhs: lhs, rhs: rhs)
    }
}

extension Data {
    func hexEncodedString() -> String {
        map { String(format: "%02hhx", $0) }.joined()
    }
}

extension SetupCmdResponse {
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

extension TapSignerResponse {
    var setupResponse: SetupCmdResponse? {
        tapSignerResponseSetupResponse(response: self)
    }

    var deriveResponse: DeriveInfo? {
        tapSignerResponseDeriveResponse(response: self)
    }

    var backupResponse: Data? {
        tapSignerResponseBackupResponse(response: self)
    }

    var isChangeResponse: Bool {
        tapSignerResponseChangeResponse(response: self)
    }
}

extension AfterPinAction {
    var userMessage: String {
        afterPinActionUserMessage(action: self)
    }
}

extension TapSignerConfirmPinArgs {
    init(from: TapSignerNewPinArgs, newPin: String) {
        self = tapSignerConfirmPinArgsNewFromNewPin(args: from, newPin: newPin)
    }
}

extension TapSigner: Equatable {
    public static func == (lhs: TapSigner, rhs: TapSigner) -> Bool {
        lhs.isEqual(rhs: rhs)
    }
}
