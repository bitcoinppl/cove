protocol WalletManagerDelegate: AnyObject {
    @MainActor
    func reconcileAfterLabelsChanged(walletId: WalletId)

    func showWalletAlert(_ alertState: AppAlertState)
}
