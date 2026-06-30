package org.bitcoinppl.cove.screenshots

import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import com.android.tools.screenshot.PreviewTest
import org.bitcoinppl.cove.cloudbackup.CloudBackupVerificationPromptFailurePreviewContent
import org.bitcoinppl.cove.cloudbackup.CloudBackupVerificationPromptPreviewContent
import org.bitcoinppl.cove.cloudbackup.CloudBackupVerificationPromptRunningPreviewContent
import org.bitcoinppl.cove.cloudbackup.CloudOnlyWalletActionSheetPreviewContent
import org.bitcoinppl.cove.flows.SettingsFlow.AboutSettingsPreviewContent
import org.bitcoinppl.cove.views.ChoiceAlertDialogNoCancelPreviewContent
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
    name = "Cloud Backup Verification Prompt Running",
    showSystemUi = true,
    widthDp = 393,
    heightDp = 852,
)
@Composable
fun CloudBackupVerificationPromptRunningScreenshot() {
    CloudBackupVerificationPromptRunningPreviewContent()
}

@PreviewTest
@Preview(
    name = "Cloud Backup Verification Prompt Failure",
    showSystemUi = true,
    widthDp = 393,
    heightDp = 852,
)
@Composable
fun CloudBackupVerificationPromptFailureScreenshot() {
    CloudBackupVerificationPromptFailurePreviewContent()
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
    name = "Choice Alert Dialog No Cancel",
    showSystemUi = true,
    widthDp = 393,
    heightDp = 852,
)
@Composable
fun ChoiceAlertDialogNoCancelScreenshot() {
    ChoiceAlertDialogNoCancelPreviewContent()
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
