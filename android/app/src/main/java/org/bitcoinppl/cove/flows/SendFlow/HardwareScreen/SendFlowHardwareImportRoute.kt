@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.HardwareScreen

import android.content.Context
import android.net.Uri
import androidx.activity.compose.ManagedActivityResultLauncher
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.SignedTransactionOrPsbt
import org.bitcoinppl.cove_core.UnsignedTransactionRecord
import org.bitcoinppl.cove_core.signedTransactionOrPsbtTryParse

internal object TransactionImportErrors {
    const val FAILED_TO_IMPORT = "Failed to import signed transaction"
    const val INVALID_HEX_FORMAT = "Invalid transaction format. Expected hexadecimal string."
    const val FILE_READ_ERROR = "Unable to read file"
    const val CLIPBOARD_EMPTY = "No text found on the clipboard."
    const val TRANSACTION_NOT_FOUND = "Transaction not found in pending transactions."
}

@Composable
internal fun rememberSignedImportFilePicker(
    app: AppManager,
    context: Context,
    onError: (String) -> Unit,
): ManagedActivityResultLauncher<String, Uri?> {
    val scope = rememberCoroutineScope()

    return rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        uri?.let {
            scope.launch {
                try {
                    val fileContents =
                        withContext(Dispatchers.IO) {
                            context.contentResolver.openInputStream(uri)?.use { input ->
                                input.bufferedReader().use { it.readText() }
                            }
                        } ?: throw Exception(TransactionImportErrors.FILE_READ_ERROR)

                    app.pushRoute(signedImportRoute(fileContents.trim()))
                } catch (e: Exception) {
                    onError(e.message ?: TransactionImportErrors.FAILED_TO_IMPORT)
                }
            }
        }
    }
}

internal fun signedImportRoute(input: String): Route {
    val (txnRecord, parsed) = parseSignedImport(input)

    return signedImportRoute(txnRecord, parsed)
}

/**
 * Parse signed import (PSBT or finalized transaction) and retrieve original unsigned transaction record
 * Returns pair of (UnsignedTransactionRecord, SignedTransactionOrPsbt)
 * Throws exception if parsing fails or transaction not found
 */
internal fun parseSignedImport(input: String): Pair<UnsignedTransactionRecord, SignedTransactionOrPsbt> {
    val parsed =
        try {
            signedTransactionOrPsbtTryParse(input)
        } catch (e: Exception) {
            throw IllegalArgumentException(TransactionImportErrors.INVALID_HEX_FORMAT, e)
        }

    val db = Database().unsignedTransactions()
    val record =
        try {
            db.getTxThrow(txId = parsed.txId())
        } catch (e: Exception) {
            throw IllegalArgumentException(TransactionImportErrors.TRANSACTION_NOT_FOUND, e)
        }

    return Pair(record, parsed)
}

private fun signedImportRoute(
    txnRecord: UnsignedTransactionRecord,
    parsed: SignedTransactionOrPsbt,
): Route =
    when (parsed) {
        is SignedTransactionOrPsbt.Transaction ->
            RouteFactory().sendConfirmSignedTransaction(
                id = txnRecord.walletId(),
                details = txnRecord.confirmDetails(),
                transaction = parsed.v1,
            )
        is SignedTransactionOrPsbt.SignedPsbt ->
            RouteFactory().sendConfirmSignedPsbt(
                id = txnRecord.walletId(),
                details = txnRecord.confirmDetails(),
                psbt = parsed.v1,
            )
    }
