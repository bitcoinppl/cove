package org.bitcoinppl.cove.views

import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** The least space a [FlexibleSpacer] keeps between sections, the spacing iOS keeps between them */
val MinSectionSpacing: Dp = 24.dp

/**
 * A gap that takes a [weight] share of its column's leftover height, as a weighted [Spacer] does,
 * but never closes below [minHeight]
 *
 * A weighted spacer shares out only leftover space, so once a screen's content outgrows the
 * screen, at a large font size or on a small phone, its gaps fall to zero and the
 * sections touch, or overflow into each other when the column can't grow. Inside a column that
 * fills the screen and scrolls, such as one with `Modifier.fillMaxSize().verticalScroll(...)`,
 * these gaps spread the sections over the screen while they fit and keep [minHeight] once they
 * don't, so the column scrolls instead
 *
 * A column only shares out height it was given as a minimum, so the column must fill the screen
 * before it scrolls. The gap lays out as two children, so a column that also spaces its children
 * with `Arrangement.spacedBy` adds that spacing twice
 */
@Composable
fun ColumnScope.FlexibleSpacer(
    weight: Float = 1f,
    minHeight: Dp = MinSectionSpacing,
) {
    Spacer(Modifier.height(minHeight))
    Spacer(Modifier.weight(weight))
}
