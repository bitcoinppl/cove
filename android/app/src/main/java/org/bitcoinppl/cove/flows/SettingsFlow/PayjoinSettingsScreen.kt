@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.PayjoinFeeHelp

internal const val PAYJOIN_FEE_HELP_UP_TO_ONE_INPUT_TAG = "payjoinSettings.feeHelp.upToOneInput"
internal const val PAYJOIN_FEE_HELP_DONT_HELP_TAG = "payjoinSettings.feeHelp.dontHelp"

private val PayjoinFeeHelp.testTag: String
    get() =
        when (this) {
            PayjoinFeeHelp.UP_TO_ONE_INPUT -> PAYJOIN_FEE_HELP_UP_TO_ONE_INPUT_TAG
            PayjoinFeeHelp.DONT_HELP -> PAYJOIN_FEE_HELP_DONT_HELP_TAG
        }

@get:StringRes
private val PayjoinFeeHelp.title: Int
    get() =
        when (this) {
            PayjoinFeeHelp.UP_TO_ONE_INPUT -> R.string.payjoin_settings_up_to_one_input
            PayjoinFeeHelp.DONT_HELP -> R.string.payjoin_settings_dont_help
        }

@get:StringRes
private val PayjoinFeeHelp.description: Int
    get() =
        when (this) {
            PayjoinFeeHelp.UP_TO_ONE_INPUT -> R.string.payjoin_settings_up_to_one_input_description
            PayjoinFeeHelp.DONT_HELP -> R.string.payjoin_settings_dont_help_description
        }

/** Settings > Payjoin, where the user picks how much a Payjoin payment offers toward the receiver's coin */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PayjoinSettingsScreen(
    app: AppManager,
    modifier: Modifier = Modifier,
) {
    val options = PayjoinFeeHelp.entries
    val selectedFeeHelp = app.payjoinFeeHelp

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = @Composable {
            SettingsTopAppBar(
                title = stringResource(R.string.title_settings_payjoin),
                onBack = { app.popRoute() },
            )
        },
        content = { paddingValues ->
            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .verticalScroll(rememberScrollState())
                        .padding(paddingValues),
            ) {
                SectionHeader(stringResource(R.string.payjoin_settings_fee_help), showDivider = false)
                MaterialSection {
                    Column {
                        options.forEachIndexed { index, feeHelp ->
                            FeeHelpRow(
                                feeHelp = feeHelp,
                                isSelected = feeHelp == selectedFeeHelp,
                                onClick = {
                                    app.dispatch(AppAction.ChangePayjoinFeeHelp(feeHelp))
                                },
                            )

                            // add divider between items, but not after the last one
                            if (index < options.size - 1) {
                                MaterialDivider()
                            }
                        }
                    }
                }
            }
        },
    )
}

@Composable
private fun FeeHelpRow(
    feeHelp: PayjoinFeeHelp,
    isSelected: Boolean,
    onClick: () -> Unit,
) {
    // selectable, so ui tests read the chosen option from its selected state
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .selectable(selected = isSelected, role = Role.RadioButton, onClick = onClick)
                .testTag(feeHelp.testTag)
                .padding(horizontal = 16.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                text = stringResource(feeHelp.title),
                style = MaterialTheme.typography.bodyMedium,
            )
            Text(
                text = stringResource(feeHelp.description),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        if (isSelected) {
            Icon(
                imageVector = Icons.Default.Check,
                contentDescription = "Selected",
                tint = MaterialTheme.colorScheme.primary,
            )
        }
    }
}
