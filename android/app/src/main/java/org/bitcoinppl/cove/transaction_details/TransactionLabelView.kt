package org.bitcoinppl.cove.transaction_details

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove_core.TransactionDetails
import org.bitcoinppl.cove_core.types.TxId

private const val TAG = "TransactionLabel"

@Composable
fun TransactionLabelView(
    transactionDetails: TransactionDetails,
    manager: WalletManager,
    secondaryColor: Color,
    snackbarHostState: SnackbarHostState,
    modifier: Modifier = Modifier,
) {
    var isEditing by remember { mutableStateOf(false) }
    var editingLabel by remember { mutableStateOf(TextFieldValue()) }
    var showMenu by remember { mutableStateOf(false) }
    var currentLabel by remember { mutableStateOf(transactionDetails.transactionLabel()) }
    var isOperationInProgress by remember { mutableStateOf(false) }
    var hasFocusedOnce by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val focusRequester = remember { FocusRequester() }
    val context = LocalContext.current

    val labelManager = remember(manager.id) { manager.rust.labelManager() }
    val txId: TxId = transactionDetails.txId()

    // update current label when transaction details change
    // guard against race condition by not updating when editing
    LaunchedEffect(transactionDetails) {
        if (!isEditing) {
            currentLabel = transactionDetails.transactionLabel()
        }
    }

    // get updated details with the new label
    fun updateDetails() {
        scope.launch {
            try {
                // bypass cache to get fresh label data from rust
                val details = manager.rust.transactionDetails(txId)
                currentLabel = details.transactionLabel()

                // update the cache so future reads get the updated label
                manager.updateTransactionDetailsCache(txId, details)
            } catch (e: Exception) {
                android.util.Log.e(TAG, "Error getting updated label", e)
                val message = context.getString(R.string.label_update_error, e.message ?: "Unknown error")
                snackbarHostState.showSnackbar(
                    message = message,
                    duration = SnackbarDuration.Short,
                )
            }
        }
    }

    fun saveLabel() {
        if (isOperationInProgress) return

        scope.launch {
            isOperationInProgress = true
            try {
                val metadata = manager.walletMetadata
                labelManager.insertOrUpdateLabelsForTxn(
                    details = transactionDetails,
                    label = editingLabel.text.trim(),
                    origin = metadata?.origin,
                )

                updateDetails()
                isEditing = false
                hasFocusedOnce = false
            } catch (e: Exception) {
                android.util.Log.e(TAG, "Unable to save label", e)
                val message = context.getString(R.string.label_save_error, e.message ?: "Unknown error")
                snackbarHostState.showSnackbar(
                    message = message,
                    duration = SnackbarDuration.Short,
                )
            } finally {
                isOperationInProgress = false
            }
        }
    }

    fun deleteLabel() {
        if (isOperationInProgress) return

        scope.launch {
            isOperationInProgress = true
            try {
                labelManager.deleteLabelsForTxn(txId = txId)
                isEditing = false
                hasFocusedOnce = false
                editingLabel = TextFieldValue()
                currentLabel = null

                updateDetails()
            } catch (e: Exception) {
                android.util.Log.e(TAG, "Unable to delete label", e)
                val message = context.getString(R.string.label_delete_error, e.message ?: "Unknown error")
                snackbarHostState.showSnackbar(
                    message = message,
                    duration = SnackbarDuration.Short,
                )
            } finally {
                isOperationInProgress = false
            }
        }
    }

    fun setEditing() {
        val text = currentLabel ?: ""
        editingLabel = TextFieldValue(text = text, selection = TextRange(text.length))
        isEditing = true
    }

    // request focus when entering edit mode
    LaunchedEffect(isEditing) {
        if (isEditing) {
            focusRequester.requestFocus()
        }
    }

    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = modifier,
    ) {
        when {
            isEditing -> {
                // editing state
                Icon(
                    imageVector = Icons.Default.Edit,
                    contentDescription = null,
                    tint = secondaryColor,
                    modifier = Modifier.size(13.dp),
                )

                Spacer(Modifier.width(8.dp))

                BasicTextField(
                    value = editingLabel,
                    onValueChange = { editingLabel = it },
                    textStyle =
                        LocalTextStyle.current.copy(
                            color = secondaryColor,
                            fontSize = 13.sp,
                        ),
                    cursorBrush = SolidColor(secondaryColor),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                    keyboardActions = KeyboardActions(onDone = { saveLabel() }),
                    modifier =
                        Modifier
                            .focusRequester(focusRequester)
                            .onFocusChanged { focusState ->
                                if (focusState.isFocused) {
                                    hasFocusedOnce = true
                                } else if (hasFocusedOnce && isEditing) {
                                    // only save when focus is actually lost (was focused, now unfocused)
                                    saveLabel()
                                }
                            },
                )

                if (isOperationInProgress) {
                    Spacer(Modifier.width(8.dp))
                    CircularProgressIndicator(
                        modifier = Modifier.size(16.dp),
                        color = secondaryColor,
                        strokeWidth = 2.dp,
                    )
                }
            }

            currentLabel != null -> {
                // has label state - show with menu
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier =
                        Modifier
                            .clip(RoundedCornerShape(8.dp))
                            .clickable(enabled = !isOperationInProgress) { showMenu = true },
                ) {
                    Icon(
                        imageVector = Icons.Default.Edit,
                        contentDescription = null,
                        tint = secondaryColor,
                        modifier = Modifier.size(16.dp),
                    )

                    Spacer(Modifier.width(8.dp))

                    Text(
                        text = currentLabel!!,
                        color = secondaryColor,
                        fontSize = 13.sp, // iOS footnote parity
                    )

                    if (isOperationInProgress) {
                        Spacer(Modifier.width(8.dp))
                        CircularProgressIndicator(
                            modifier = Modifier.size(16.dp),
                            color = secondaryColor,
                            strokeWidth = 2.dp,
                        )
                    }
                }

                // dropdown menu for edit/delete
                DropdownMenu(
                    expanded = showMenu,
                    onDismissRequest = { showMenu = false },
                ) {
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.btn_edit_label)) },
                        onClick = {
                            showMenu = false
                            setEditing()
                        },
                        enabled = !isOperationInProgress,
                        leadingIcon = {
                            Icon(
                                imageVector = Icons.Default.Edit,
                                contentDescription = null,
                            )
                        },
                    )

                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.btn_delete_label)) },
                        onClick = {
                            showMenu = false
                            deleteLabel()
                        },
                        enabled = !isOperationInProgress,
                        leadingIcon = {
                            Icon(
                                imageVector = Icons.Default.Delete,
                                contentDescription = null,
                            )
                        },
                    )
                }
            }

            else -> {
                // no label state - show add button
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier =
                        Modifier
                            .clip(RoundedCornerShape(8.dp))
                            .clickable(enabled = !isOperationInProgress) { setEditing() },
                ) {
                    Icon(
                        imageVector = Icons.Default.Add,
                        contentDescription = null,
                        tint = secondaryColor,
                    )

                    Spacer(Modifier.width(8.dp))

                    Text(
                        text = stringResource(R.string.btn_add_label),
                        color = secondaryColor,
                        fontSize = 13.sp, // iOS footnote parity
                    )
                }
            }
        }
    }
}
