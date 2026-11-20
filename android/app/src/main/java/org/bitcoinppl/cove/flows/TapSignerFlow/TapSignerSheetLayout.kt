@file:Suppress("PackageNaming") // package name matches the existing TapSignerFlow namespace

package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import org.bitcoinppl.cove.views.FlexibleSpacer

/**
 * Lays out a TapSigner sheet: [header] at the top, [action] at the bottom, and [content] centred
 * between them in a column with [contentModifier], tagged `<testTag>.header` and
 * `<testTag>.content`
 *
 * While everything fits, the sheet fills its height as a fixed layout would. Once it doesn't, at
 * a large font size or on a small phone, the sheet scrolls and keeps its sections apart
 * instead of letting the content overflow into the header and the action, as iOS's
 * `TapSignerAdaptiveLayout` does
 */
// the three slots and two modifiers are the layout's API, so grouping them would only add a wrapper
@Suppress("LongParameterList")
@Composable
internal fun TapSignerSheetLayout(
    testTag: String,
    header: @Composable () -> Unit,
    content: @Composable ColumnScope.() -> Unit,
    action: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    contentModifier: Modifier = Modifier,
) {
    Column(
        modifier =
            modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState()),
    ) {
        Box(Modifier.fillMaxWidth().testTag("$testTag.header")) {
            header()
        }

        FlexibleSpacer()

        Column(
            modifier = contentModifier.fillMaxWidth().testTag("$testTag.content"),
            horizontalAlignment = Alignment.CenterHorizontally,
            content = content,
        )

        FlexibleSpacer()

        action()
    }
}
