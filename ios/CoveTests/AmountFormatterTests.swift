@testable import Cove
import CoveCore
import XCTest

final class AmountFormatterTests: XCTestCase {
    func testBtcAmountFormattingMatchesExistingHelpers() {
        var metadata = walletMetadataPreview()
        metadata.selectedUnit = .btc
        let amount = Amount.fromSat(sats: 123_456_789)
        let formatter = AmountFormatter(metadata: metadata)

        XCTAssertEqual(formatter.amountFmt(amount), amount.btcString())
        XCTAssertEqual(
            formatter.displayAmount(amount),
            walletDisplayAmount(metadata: metadata, amount: amount, showUnit: true)
        )
        XCTAssertEqual(
            formatter.displayAmount(amount, showUnit: false),
            walletDisplayAmount(metadata: metadata, amount: amount, showUnit: false)
        )
        XCTAssertEqual(formatter.amountFmtUnit(amount), amount.btcStringWithUnit())
    }

    func testSatAmountFormattingMatchesExistingHelpers() {
        var metadata = walletMetadataPreview()
        metadata.selectedUnit = .sat
        let amount = Amount.fromSat(sats: 123_456_789)
        let formatter = AmountFormatter(metadata: metadata)

        XCTAssertEqual(formatter.amountFmt(amount), amount.satsString())
        XCTAssertEqual(
            formatter.displayAmountWithDirection(amount, direction: .incoming),
            walletDisplayAmountWithDirection(
                metadata: metadata,
                amount: amount,
                direction: .incoming
            )
        )
        XCTAssertEqual(
            formatter.displayAmountPendingFmt(amount),
            walletDisplayAmountPendingFmt(metadata: metadata, amount: amount)
        )
        XCTAssertEqual(formatter.amountFmtUnit(amount), amount.satsStringWithUnit())
    }

    func testFiatFormattingMatchesExistingHelpers() {
        var metadata = walletMetadataPreview()
        metadata.sensitiveVisible = true
        let formatter = AmountFormatter(metadata: metadata)
        let amount = 1234.56

        XCTAssertEqual(
            formatter.displayFiatAmount(amount),
            walletDisplayFiatAmount(metadata: metadata, amount: amount, withSuffix: true)
        )
        XCTAssertEqual(
            formatter.displayFiatAmount(amount, withSuffix: false),
            walletDisplayFiatAmount(metadata: metadata, amount: amount, withSuffix: false)
        )
        XCTAssertEqual(
            formatter.displayFiatAmountWithDirection(amount, direction: .outgoing),
            walletDisplayFiatAmountWithDirection(
                metadata: metadata,
                amount: amount,
                direction: .outgoing,
                withSuffix: true
            )
        )
        XCTAssertEqual(
            formatter.displayFiatAmountPendingFmt(amount),
            walletDisplayFiatAmountPendingFmt(metadata: metadata, amount: amount, withSuffix: true)
        )
    }

    func testSentAndReceivedFormattingMatchesExistingHelpers() {
        let metadata = walletMetadataPreview()
        let transaction = transactionsPreviewNew(confirmed: 0, unconfirmed: 1)[0]
        let sentAndReceived = transaction.sentAndReceived()
        let formatter = AmountFormatter(metadata: metadata)

        XCTAssertEqual(
            formatter.displaySentAndReceivedAmount(sentAndReceived),
            walletDisplaySentAndReceivedAmount(
                metadata: metadata,
                sentAndReceived: sentAndReceived
            )
        )
    }
}
