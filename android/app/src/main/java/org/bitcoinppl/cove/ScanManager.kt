package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.tapcard.*
import org.bitcoinppl.cove_core.types.*

@Stable
class ScanManager private constructor() {
    private val tag = "ScanManager"
    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    private val app: AppManager get() = AppManager.getInstance()

    fun handleMultiFormat(multiFormat: MultiFormat) {
        try {
            when (multiFormat) {
                is MultiFormat.Mnemonic -> {
                    multiFormat.v1.use { mnemonic ->
                        importHotWallet(mnemonic.words())
                    }
                }

                is MultiFormat.HardwareExport -> {
                    importColdWallet(multiFormat.v1)
                }

                is MultiFormat.Address -> {
                    handleAddress(multiFormat.v1)
                }

                is MultiFormat.Transaction -> {
                    handleTransaction(multiFormat.v1)
                }

                is MultiFormat.SignedPsbt -> {
                    handleSignedPsbt(multiFormat.v1)
                }

                is MultiFormat.TapSignerUnused -> {
                    app.alertState = TaggedItem(AppAlertState.UninitializedTapSigner(multiFormat.v1))
                }

                is MultiFormat.TapSignerReady -> {
                    val wallet = app.findTapSignerWallet(multiFormat.v1)
                    if (wallet != null) {
                        app.alertState = TaggedItem(AppAlertState.TapSignerWalletFound(wallet.id))
                    } else {
                        app.alertState = TaggedItem(AppAlertState.InitializedTapSigner(multiFormat.v1))
                    }
                }

                is MultiFormat.Bip329Labels -> {
                    val selectedWallet = Database().globalConfig().selectedWallet()
                    if (selectedWallet == null) {
                        app.alertState =
                            TaggedItem(
                                AppAlertState.InvalidFileFormat(
                                    "Currently BIP329 labels must be imported through the wallet actions",
                                ),
                            )
                        return
                    }

                    try {
                        LabelManager(id = selectedWallet).use { it.importLabels(multiFormat.v1) }
                        app.alertState = TaggedItem(AppAlertState.ImportedLabelsSuccessfully)

                        app.walletManager?.let { wm ->
                            mainScope.launch {
                                wm.rust.getTransactions()
                            }
                        }
                    } catch (e: Exception) {
                        Log.e(tag, "Failed to import labels", e)
                        app.alertState =
                            TaggedItem(
                                AppAlertState.InvalidFileFormat(
                                    e.message ?: "Failed to import labels",
                                ),
                            )
                    }
                }
            }
        } catch (e: Exception) {
            Log.e(tag, "Unable to handle scanned code", e)
            app.alertState =
                TaggedItem(
                    AppAlertState.InvalidFileFormat(e.message ?: "Unknown error"),
                )
        }
    }

    private fun importHotWallet(words: List<String>) {
        val manager = ImportWalletManager()
        try {
            val walletMetadata = manager.rust.importWallet(listOf(words))
            app.rust.selectWallet(walletMetadata.id)
        } catch (e: ImportWalletException.InvalidWordGroup) {
            Log.d(tag, "Invalid word group detected")
            app.alertState = TaggedItem(AppAlertState.InvalidWordGroup)
        } catch (e: ImportWalletException.WalletAlreadyExists) {
            Log.w(tag, "Attempted to import words for an existing hot wallet: ${e.v1}")
            app.alertState = TaggedItem(AppAlertState.DuplicateWallet(e.v1))
            try {
                app.rust.selectWallet(e.v1)
            } catch (selectError: Exception) {
                Log.e(tag, "Unable to select existing wallet", selectError)
            }
        } catch (e: Exception) {
            Log.e(tag, "Unable to import wallet", e)
            app.alertState =
                TaggedItem(
                    AppAlertState.ErrorImportingHotWallet(e.message ?: "Unknown error"),
                )
        } finally {
            manager.close()
        }
    }

    private fun importColdWallet(export: HardwareExport) {
        try {
            val wallet = Wallet.newFromExport(export)
            try {
                val id = wallet.id()
                Log.d(tag, "Imported Wallet: $id")
                app.alertState = TaggedItem(AppAlertState.ImportedSuccessfully)

                if (app.walletManager?.id != id) {
                    app.rust.selectWallet(id)
                }

                if (app.walletManager?.id == id && app.walletManager?.walletMetadata?.walletType != WalletType.HOT) {
                    try {
                        app.walletManager?.rust?.setWalletType(WalletType.COLD)
                    } catch (e: Exception) {
                        Log.e(tag, "Failed to set wallet type to cold", e)
                    }
                }
            } finally {
                wallet.close()
            }
        } catch (e: WalletException.WalletAlreadyExists) {
            app.alertState = TaggedItem(AppAlertState.DuplicateWallet(e.v1))
            try {
                app.rust.selectWallet(e.v1)
            } catch (selectError: Exception) {
                Log.e(tag, "Unable to select existing wallet", selectError)
                app.alertState = TaggedItem(AppAlertState.UnableToSelectWallet)
            }
        } catch (e: Exception) {
            Log.e(tag, "Error importing hardware wallet", e)
            app.alertState =
                TaggedItem(
                    AppAlertState.ErrorImportingHardwareWallet(e.message ?: "Unknown error"),
                )
        }
    }

    private fun handleAddress(addressWithNetwork: AddressWithNetwork) {
        val db = Database()
        val currentNetwork = db.globalConfig().selectedNetwork()
        val address = addressWithNetwork.address()
        val network = addressWithNetwork.network()
        val selectedWallet = db.globalConfig().selectedWallet()

        if (selectedWallet == null) {
            app.alertState = TaggedItem(AppAlertState.NoWalletSelected(address))
            return
        }

        if (!addressWithNetwork.isValidForNetwork(currentNetwork)) {
            app.alertState =
                TaggedItem(
                    AppAlertState.AddressWrongNetwork(
                        address = address,
                        network = network,
                        currentNetwork = currentNetwork,
                    ),
                )
            return
        }

        val amount = addressWithNetwork.amount()
        app.alertState = TaggedItem(AppAlertState.FoundAddress(address, amount))
    }

    private fun handleTransaction(transaction: BitcoinTransaction) {
        Log.d(tag, "Received BitcoinTransaction: $transaction: ${transaction.txIdHash()}")

        val db = Database().unsignedTransactions()
        val txnRecord = db.getTx(transaction.txId())

        if (txnRecord == null) {
            Log.e(tag, "No unsigned transaction found for ${transaction.txId()}")
            app.alertState = TaggedItem(AppAlertState.NoUnsignedTransactionFound(transaction.txId()))
            return
        }

        val route =
            RouteFactory().sendConfirm(
                id = txnRecord.walletId(),
                details = txnRecord.confirmDetails(),
                signedTransaction = transaction,
            )

        app.pushRoute(route)
    }

    private fun handleSignedPsbt(psbt: Psbt) {
        Log.d(tag, "Received signed PSBT: ${psbt.txId()}")

        val db = Database().unsignedTransactions()
        val txnRecord = db.getTx(psbt.txId())

        if (txnRecord == null) {
            Log.e(tag, "No unsigned transaction found for PSBT ${psbt.txId()}")
            app.alertState = TaggedItem(AppAlertState.NoUnsignedTransactionFound(psbt.txId()))
            return
        }

        val route =
            RouteFactory().sendConfirm(
                id = txnRecord.walletId(),
                details = txnRecord.confirmDetails(),
                signedPsbt = psbt,
            )

        app.pushRoute(route)
    }

    companion object {
        @Volatile
        private var instance: ScanManager? = null

        fun getInstance(): ScanManager =
            instance ?: synchronized(this) {
                instance ?: ScanManager().also { instance = it }
            }
    }
}

val Scanner: ScanManager
    get() = ScanManager.getInstance()
