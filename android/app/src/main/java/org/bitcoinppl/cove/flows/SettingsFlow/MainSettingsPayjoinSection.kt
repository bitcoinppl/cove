package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.JoinInner
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.res.stringResource
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.DatabaseException
import org.bitcoinppl.cove_core.PayjoinSetting

private const val TAG = "PayjoinSection"

/** The Payjoin switch, shown only when this build's Payjoin rollout offers it */
@Composable
internal fun PayjoinSection() {
    var setting by remember { mutableStateOf(Database().globalConfig().payjoinSetting()) }
    val available = setting as? PayjoinSetting.Available ?: return

    if (available.beta) {
        BetaSectionHeader(title = stringResource(R.string.title_settings_payments))
    } else {
        SectionHeader(stringResource(R.string.title_settings_payments))
    }

    MaterialSection {
        MaterialSettingsItem(
            title = stringResource(R.string.title_settings_payjoin),
            subtitle = stringResource(R.string.settings_payjoin_subtitle),
            icon = Icons.Default.JoinInner,
            isSwitch = true,
            switchCheckedState = available.enabled,
            onCheckChanged = { enabled ->
                try {
                    Database().globalConfig().setPayjoinEnabled(enabled)
                } catch (e: DatabaseException) {
                    Log.e(TAG, "Failed to save the Payjoin choice", e)
                }

                // read back what was saved, so a failed save leaves the switch where it was
                setting = Database().globalConfig().payjoinSetting()
            },
        )
    }
}
