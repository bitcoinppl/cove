package org.bitcoinppl.cove.flows.SendFlow

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
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.SignedTransactionOrPsbt
import org.bitcoinppl.cove_core.UnsignedTransactionRecord
import org.bitcoinppl.cove_core.signedTransactionOrPsbtTryParse

internal enum class TransactionImportError {
    InvalidFormat,
    FileRead,
    TransactionNotFound,
}

internal class TransactionImportException(
    val importError: TransactionImportError,
    cause: Throwable? = null,
) : Exception(null, cause)

internal fun Throwable.signedImportErrorStringRes(): Int =
    when ((this as? TransactionImportException)?.importError) {
        TransactionImportError.InvalidFormat -> R.string.wallet_send_invalid_transaction_format
        TransactionImportError.FileRead -> R.string.wallet_send_unable_to_read_file
        TransactionImportError.TransactionNotFound -> R.string.wallet_send_transaction_not_found
        null -> R.string.wallet_send_failed_import_signed_transaction
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
                        } ?: throw TransactionImportException(TransactionImportError.FileRead)

                    app.pushRoute(signedImportRoute(fileContents.trim()))
                } catch (e: Exception) {
                    onError(context.getString(e.signedImportErrorStringRes()))
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
            throw TransactionImportException(TransactionImportError.InvalidFormat, e)
        }

    val db = Database().unsignedTransactions()
    val record =
        try {
            db.getTxThrow(txId = parsed.txId())
        } catch (e: Exception) {
            throw TransactionImportException(TransactionImportError.TransactionNotFound, e)
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
