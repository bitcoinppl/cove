struct AmountFormatter {
    let metadata: WalletMetadata

    func amountFmt(_ amount: Amount) -> String {
        switch metadata.selectedUnit {
        case .btc:
            amount.btcString()
        case .sat:
            amount.satsString()
        }
    }

    func displayAmount(_ amount: Amount, showUnit: Bool = true) -> String {
        walletDisplayAmount(metadata: metadata, amount: amount, showUnit: showUnit)
    }

    func displayAmountPendingFmt(_ amount: Amount) -> String? {
        walletDisplayAmountPendingFmt(metadata: metadata, amount: amount)
    }

    func displayAmountWithDirection(
        _ amount: Amount,
        direction: TransactionDirection
    ) -> String {
        walletDisplayAmountWithDirection(
            metadata: metadata,
            amount: amount,
            direction: direction
        )
    }

    func displaySentAndReceivedAmount(_ sentAndReceived: SentAndReceived) -> String {
        walletDisplaySentAndReceivedAmount(
            metadata: metadata,
            sentAndReceived: sentAndReceived
        )
    }

    func displayFiatAmount(_ amount: Double, withSuffix: Bool = true) -> String {
        walletDisplayFiatAmount(
            metadata: metadata,
            amount: amount,
            withSuffix: withSuffix
        )
    }

    func displayFiatAmountPendingFmt(
        _ amount: Double,
        withSuffix: Bool = true
    ) -> String? {
        walletDisplayFiatAmountPendingFmt(
            metadata: metadata,
            amount: amount,
            withSuffix: withSuffix
        )
    }

    func displayFiatAmountWithDirection(
        _ amount: Double,
        direction: TransactionDirection,
        withSuffix: Bool = true
    ) -> String {
        walletDisplayFiatAmountWithDirection(
            metadata: metadata,
            amount: amount,
            direction: direction,
            withSuffix: withSuffix
        )
    }

    func amountInFiatCached(_ amount: Amount) -> Double? {
        walletAmountInFiatCached(amount: amount)
    }

    func amountFmtUnit(_ amount: Amount) -> String {
        switch metadata.selectedUnit {
        case .btc: amount.btcStringWithUnit()
        case .sat: amount.satsStringWithUnit()
        }
    }
}
