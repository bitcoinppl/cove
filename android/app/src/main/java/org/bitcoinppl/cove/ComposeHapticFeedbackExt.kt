package org.bitcoinppl.cove

import androidx.compose.ui.hapticfeedback.HapticFeedback as ComposeHapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType

fun ComposeHapticFeedback.performWalletReorderHaptic() {
    performHapticFeedback(HapticFeedbackType.Confirm)
}
