package org.bitcoinppl.cove.screenshots

import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import com.android.tools.screenshot.PreviewTest
import org.bitcoinppl.cove.cloudbackup.CloudBackupVerificationPromptPreviewContent
import org.bitcoinppl.cove.cloudbackup.CloudOnlyWalletActionSheetPreviewContent
import org.bitcoinppl.cove.flows.SettingsFlow.AboutSettingsPreviewContent
import org.bitcoinppl.cove.views.ChoiceAlertDialogPreviewContent

@PreviewTest
@Preview(
    name = "About Settings",
    showSystemUi = true,
    widthDp = 393,
    heightDp = 852,
)
@Composable
fun AboutSettingsPreviewScreenshot() {
    AboutSettingsPreviewContent()
}

@PreviewTest
@Preview(
    name = "Cloud Backup Verification Prompt",
    showSystemUi = true,
    widthDp = 393,
    heightDp = 852,
)
@Composable
fun CloudBackupVerificationPromptScreenshot() {
    CloudBackupVerificationPromptPreviewContent()
}

@PreviewTest
@Preview(
    name = "Choice Alert Dialog",
    showSystemUi = true,
    widthDp = 393,
    heightDp = 852,
)
@Composable
fun ChoiceAlertDialogScreenshot() {
    ChoiceAlertDialogPreviewContent()
}

@PreviewTest
@Preview(
    name = "Cloud Only Wallet Action Sheet",
    showSystemUi = true,
    widthDp = 393,
    heightDp = 852,
)
@Composable
fun CloudOnlyWalletActionSheetScreenshot() {
    CloudOnlyWalletActionSheetPreviewContent()
}
