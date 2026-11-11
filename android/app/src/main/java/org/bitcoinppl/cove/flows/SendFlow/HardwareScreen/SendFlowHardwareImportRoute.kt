@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.HardwareScreen

import android.content.Context
import android.net.Uri
import androidx.activity.compose.ManagedActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.views.rememberLockAwareLauncherForActivityResult
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.SignedTransactionOrPsbt
import org.bitcoinppl.cove_core.UnsignedTransactionRecord
import org.bitcoinppl.cove_core.signedTransactionOrPsbtTryFromBytes
import org.bitcoinppl.cove_core.signedTransactionOrPsbtTryParse

private const val TAG = "SignedImport"

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

    return rememberLockAwareLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        uri?.let {
            scope.launch {
                try {
                    // signers write binary PSBTs and transactions, so Rust detects the format from raw bytes
                    val fileData =
                        withContext(Dispatchers.IO) { readSignedImportFile(context, uri) }
                            ?: throw Exception(TransactionImportErrors.FILE_READ_ERROR)

                    app.pushRoute(signedImportRoute(fileData))
                } catch (e: Exception) {
                    onError(e.message ?: TransactionImportErrors.FAILED_TO_IMPORT)
                }
            }
        }
    }
}

/** Reads the picked file, logging why it could not be read and returning null */
private fun readSignedImportFile(
    context: Context,
    uri: Uri,
): ByteArray? =
    try {
        val data = context.contentResolver.openInputStream(uri)?.use { it.readBytes() }
        if (data == null) Log.w(TAG, "Signed import file picker returned no readable stream")

        data
    } catch (e: IOException) {
        // the exception message can name the picked file, so log only the kind
        Log.w(TAG, "Signed import file read failed: ${e.javaClass.simpleName}")
        null
    } catch (e: SecurityException) {
        Log.w(TAG, "Signed import file access denied: ${e.javaClass.simpleName}")
        null
    }

internal fun signedImportRoute(input: String): Route {
    val (txnRecord, parsed) = parseSignedImport { signedTransactionOrPsbtTryParse(input) }

    return signedImportRoute(txnRecord, parsed)
}

/** Route for a signed import file's raw bytes, which may be a binary or text PSBT or transaction */
internal fun signedImportRoute(data: ByteArray): Route {
    val (txnRecord, parsed) = parseSignedImport { signedTransactionOrPsbtTryFromBytes(data) }

    return signedImportRoute(txnRecord, parsed)
}

/**
 * Parse signed import (PSBT or finalized transaction) and retrieve original unsigned transaction record
 * Returns pair of (UnsignedTransactionRecord, SignedTransactionOrPsbt)
 * Throws exception if parsing fails or transaction not found
 */
private fun parseSignedImport(
    parse: () -> SignedTransactionOrPsbt,
): Pair<UnsignedTransactionRecord, SignedTransactionOrPsbt> {
    val parsed = parseSignedInput(parse)
    val record = unsignedRecordFor(parsed)

    return Pair(record, parsed)
}

private fun parseSignedInput(parse: () -> SignedTransactionOrPsbt): SignedTransactionOrPsbt =
    try {
        parse()
    } catch (e: Exception) {
        // the input is a PSBT or transaction and the message can quote it, so log only the kind
        Log.w(TAG, "Signed import parse failed: ${e.javaClass.simpleName}")
        throw IllegalArgumentException(TransactionImportErrors.INVALID_HEX_FORMAT, e)
    }

private fun unsignedRecordFor(parsed: SignedTransactionOrPsbt): UnsignedTransactionRecord =
    try {
        Database().unsignedTransactions().getTxThrow(txId = parsed.unsignedTxId())
    } catch (e: Exception) {
        Log.w(TAG, "Signed import has no matching unsigned transaction: ${e.javaClass.simpleName}")
        throw IllegalArgumentException(TransactionImportErrors.TRANSACTION_NOT_FOUND, e)
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
