package org.bitcoinppl.cove.screenshots

import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import com.android.tools.screenshot.PreviewTest
import org.bitcoinppl.cove.flows.SettingsFlow.AboutSettingsPreviewContent

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
