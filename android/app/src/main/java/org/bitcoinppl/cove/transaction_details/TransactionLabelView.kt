package org.bitcoinppl.cove.transaction_details

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalTextStyle
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove_core.TransactionDetails
import org.bitcoinppl.cove_core.types.TxId

@Composable
fun TransactionLabelView(
    transactionDetails: TransactionDetails,
    manager: WalletManager,
    secondaryColor: Color,
    modifier: Modifier = Modifier,
) {
    var isEditing by remember { mutableStateOf(false) }
    var editingLabel by remember { mutableStateOf("") }
    var showMenu by remember { mutableStateOf(false) }
    var currentLabel by remember { mutableStateOf(transactionDetails.transactionLabel()) }
    val scope = rememberCoroutineScope()
    val focusRequester = remember { FocusRequester() }

    val labelManager = remember { manager.rust.labelManager() }
    val txId: TxId = transactionDetails.txId()

    // update current label when transaction details change
    LaunchedEffect(transactionDetails) {
        currentLabel = transactionDetails.transactionLabel()
    }

    // get updated details with the new label
    fun updateDetails() {
        scope.launch {
            try {
                val details = manager.transactionDetails(txId = txId)
                currentLabel = details.transactionLabel()
            } catch (e: Exception) {
                println("Error getting updated label: $e")
            }
        }
    }

    fun saveLabel() {
        if (editingLabel.isBlank()) {
            isEditing = false
            return
        }

        scope.launch {
            try {
                val metadata = manager.walletMetadata
                labelManager.insertOrUpdateLabelsForTxn(
                    details = transactionDetails,
                    label = editingLabel,
                    origin = metadata?.origin,
                )

                updateDetails()
                isEditing = false
            } catch (e: Exception) {
                println("Unable to save label: $e")
            }
        }
    }

    fun deleteLabel() {
        scope.launch {
            try {
                labelManager.deleteLabelsForTxn(txId = txId)
                isEditing = false
                editingLabel = ""
                currentLabel = null

                updateDetails()
            } catch (e: Exception) {
                println("Unable to delete label: $e")
            }
        }
    }

    fun setEditing() {
        editingLabel = currentLabel ?: ""
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
                )

                Spacer(Modifier.width(8.dp))

                BasicTextField(
                    value = editingLabel,
                    onValueChange = { editingLabel = it },
                    textStyle =
                        LocalTextStyle.current.copy(
                            color = secondaryColor,
                            fontSize = 14.sp,
                        ),
                    cursorBrush = SolidColor(secondaryColor),
                    modifier =
                        Modifier
                            .focusRequester(focusRequester)
                            .onFocusChanged { focusState ->
                                // auto-save when focus is lost
                                if (!focusState.isFocused && isEditing) {
                                    saveLabel()
                                }
                            },
                )
            }

            currentLabel != null -> {
                // has label state - show with menu
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier =
                        Modifier
                            .clip(RoundedCornerShape(8.dp))
                            .clickable { showMenu = true },
                ) {
                    Icon(
                        imageVector = Icons.Default.Edit,
                        contentDescription = null,
                        tint = secondaryColor,
                    )

                    Spacer(Modifier.width(8.dp))

                    Text(
                        text = currentLabel!!,
                        color = secondaryColor,
                        fontSize = 14.sp,
                    )
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
                            .clickable { setEditing() },
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
                        fontSize = 14.sp,
                    )
                }
            }
        }
    }
}
