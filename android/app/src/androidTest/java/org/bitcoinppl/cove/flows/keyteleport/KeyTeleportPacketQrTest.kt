package org.bitcoinppl.cove.flows.keyteleport

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.bitcoinppl.cove.test.coveComposeRule
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * KeyTeleport's packet QR must keep the four-module white quiet zone that QR scanners look for,
 * since it shows on a dark screen and scanners such as the Coldcard Q's find no code without it
 */
@RunWith(AndroidJUnit4::class)
class KeyTeleportPacketQrTest {
    private companion object {
        /** The receiver request from the Rust KeyTeleport tests */
        const val RECEIVER_PACKET = "B\$2R0100VHT2AGUUH7KUZUUSTOWOIWHJX3XM7GA2N4BHQOXDFHXLVHVA7K6ZO"

        /** The quiet zone, in modules, that the QR code standard asks for */
        const val QUIET_ZONE_MODULES = 4f
    }

    @get:Rule
    val compose = coveComposeRule()

    @Test
    fun packetQrKeepsAFourModuleQuietZone() {
        // red around the card, so the white quiet zone ends where the red begins
        compose.setContent {
            Box(Modifier.background(Color.Red).padding(16.dp)) { PacketQr(RECEIVER_PACKET) }
        }

        val pixels = compose.onRoot().captureToImage().toPixelMap()
        val isDark = { x: Int, y: Int -> pixels[x, y].let { maxOf(it.red, it.green, it.blue) < 0.5f } }
        val isBackground = { x: Int, y: Int -> pixels[x, y].let { it.red > 0.8f && it.green < 0.3f && it.blue < 0.3f } }
        // the scaled bitmap blends the code's edges, so any pixel short of dark is the card's border
        val isBorder = { x: Int, y: Int -> !isDark(x, y) && !isBackground(x, y) }

        // the top row of the two upper finder patterns, each 7 modules wide
        val top = (0 until pixels.height).first { y -> (0 until pixels.width).any { x -> isDark(x, y) } }
        val left = (0 until pixels.width).first { x -> isDark(x, top) }
        val right = (pixels.width - 1 downTo 0).first { x -> isDark(x, top) }
        val finderWidth = (left until pixels.width).takeWhile { x -> isDark(x, top) }.count()
        val module = finderWidth / 7f
        val finderMiddle = left + finderWidth / 2

        // border pixels between each edge of the code and the red around the card
        val quietZones =
            mapOf(
                "top" to (top - 1 downTo 0).takeWhile { y -> isBorder(finderMiddle, y) }.count(),
                "left" to (left - 1 downTo 0).takeWhile { x -> isBorder(x, top) }.count(),
                "right" to (right + 1 until pixels.width).takeWhile { x -> isBorder(x, top) }.count(),
            )

        quietZones.forEach { (side, pixelsWide) ->
            val modules = pixelsWide / module
            val measured = "the code's top left at ($left, $top), its finder pattern $finderWidth pixels wide"
            assertTrue("the QR's $side quiet zone is $modules modules, under $QUIET_ZONE_MODULES; $measured", modules >= QUIET_ZONE_MODULES)
        }
    }
}
