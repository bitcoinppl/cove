package org.bitcoinppl.cove.flows.keyteleport

import android.graphics.Bitmap
import android.util.Log
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.roundToIntRect
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.NotFoundException
import com.google.zxing.RGBLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.test.TestWalletWords
import org.bitcoinppl.cove.test.TextReferenceMatch
import org.bitcoinppl.cove.test.awaitOnMain
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.deleteTestWallet
import org.bitcoinppl.cove.test.importTestWalletOrNull
import org.bitcoinppl.cove.test.matchTextReference
import org.bitcoinppl.cove.test.textLayouts
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.bitcoinppl.cove_core.KeyTeleportRoute
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import kotlin.math.abs

/**
 * The receiver request and the sender response must show their QR code and their code or password
 * one at a time on screen, not only in the accessibility tree
 *
 * Hosts the real KeyTeleport screen, whose window is protected with FLAG_SECURE, and reads what it
 * drew from in-app captures, as the iOS tests do from screenshots. zxing must decode the shown QR
 * code to the exact packet and find no QR code while it is hidden. The shown code must match a
 * reference rendering of its own text and the hidden one must not. Without OCR, the hidden code
 * must also carry almost nothing of what tells two secrets apart: two transfers' hidden codes may
 * differ by only a small share of how much the same codes differ once shown
 *
 * Secrets never go into assertion messages, which end up in test reports
 */
class KeyTeleportRevealCaptureTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val app get() = AppManager.getInstance()
    private var walletId: WalletId? = null

    @Before
    fun setUp() {
        bootstrapRustRuntimeForUiTest()

        // both tests run a receive, and ending it deletes the stored session, so never run over someone's pending transfer
        assumeTrue("a KeyTeleport receive session is already stored on this device", !hasStoredReceiveSession())
    }

    @After
    fun tearDown() {
        try {
            if (hasStoredReceiveSession()) endReceive(app.getKeyTeleportManager())
        } finally {
            compose.runOnUiThread { app.clearKeyTeleportManager() }
            walletId?.let(::deleteTestWallet)
        }
    }

    @Test
    fun receiverRequestShowsOnlyOneOfItsQrCodeAndReceiverCode() {
        showKeyTeleport(KeyTeleportRoute.RECEIVE)
        val receiver = app.getKeyTeleportManager()

        val first = awaitRequest(receiver)
        val firstWalk = walkRevealPair("the first receiver request", first.packetText, first.groupedNumericCode, RECEIVER_CODE_HINT)

        receiver.dispatch(KeyTeleportManagerAction.RestartReceive)
        val second =
            awaitOnMain("a new receiver request") {
                (receiver.state as? KeyTeleportManagerState.ReceiveReady)?.v1?.takeIf { it.packet.bbqrPart() != first.packetText }
            }
        val secondWalk = walkRevealPair("the second receiver request", second.packet.bbqrPart(), second.groupedNumericCode, RECEIVER_CODE_HINT)

        assertOneValueReadableAtATime("receiver code", firstWalk, secondWalk)
        endReceive(receiver)
    }

    @Test
    fun senderResponseShowsOnlyOneOfItsQrCodeAndPassword() {
        val imported = importTestWalletOrNull(TEST_WALLET)
        assumeTrue("the test wallet already exists on this device", imported != null)
        val wallet = requireNotNull(imported).id
        walletId = wallet

        showKeyTeleport(KeyTeleportRoute.SEND)
        val sender = app.getKeyTeleportManager()

        KeyTeleportTestPeer().use { receiver ->
            val request = receiver.startReceive()

            val first = sendTo(sender, wallet, request, previousPassword = null)
            val firstWalk = walkRevealPair("the first sender response", first.packetText, first.password, SENDER_PASSWORD_HINT)

            val second = sendTo(sender, wallet, request, previousPassword = first.password)
            val secondWalk = walkRevealPair("the second sender response", second.packetText, second.password, SENDER_PASSWORD_HINT)

            assertOneValueReadableAtATime("password", firstWalk, secondWalk)
            sender.dispatch(KeyTeleportManagerAction.Clear)
        }
    }

    private fun showKeyTeleport(route: KeyTeleportRoute) {
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                KeyTeleportContainer(app = app, route = route)
            }
        }
        compose.waitForIdle()
    }

    // drives the screen's own manager as the peer drives its own, standing in for the camera scan
    private fun sendTo(
        sender: KeyTeleportManager,
        wallet: WalletId,
        request: KeyTeleportRequestCopy,
        previousPassword: String?,
    ): KeyTeleportResponseCopy {
        sender.dispatch(KeyTeleportManagerAction.StartSendFromWallet(wallet))
        awaitOnMain("the sender to wait for a receiver request") {
            sender.state.takeIf { it is KeyTeleportManagerState.SendAwaitReceiver }
        }

        val accepted =
            awaitOnMain("the sender to read the receiver request") {
                sender.ingestKeyTeleportText(request.packetText, KeyTeleportFlowDirection.SEND)
            }

        assertTrue("the sender rejected the receiver request", accepted)
        awaitOnMain("the sender to ask for the receiver code") {
            sender.state.takeIf { it is KeyTeleportManagerState.SendEnterCode }
        }

        sender.dispatch(KeyTeleportManagerAction.EnterReceiverCode(request.numericCode))
        // the previous response stays on screen until the new one replaces it
        return awaitOnMain("a new sender response") {
            (sender.state as? KeyTeleportManagerState.SendReady)
                ?.v1
                ?.let { KeyTeleportResponseCopy(packetText = it.packet.bbqrPart(), password = it.password.groupedText()) }
                ?.takeIf { it.password != previousPassword }
        }
    }

    /**
     * Capture the reveal pair before anything is revealed, with the code shown, and with the QR code
     * shown again
     *
     * Checks come after both walks, so a failing run still logs every measurement
     */
    private fun walkRevealPair(
        screen: String,
        packet: String,
        code: String,
        codeHint: String,
    ): RevealWalk {
        compose.waitForIdle()
        assertTrue("$screen is not protected with FLAG_SECURE", windowIsSecure())

        val codeRegion = compose.onNodeWithContentDescription(codeHint).regionInRoot()
        val qrFirst = capture()

        compose.onNodeWithContentDescription(codeHint).performClick()
        compose.waitForIdle()
        val codeShown = capture()
        val qrRegion = compose.onNodeWithContentDescription(QR_HINT).regionInRoot()
        val codeNode = compose.onNode(hasText(code), useUnmergedTree = true).fetchSemanticsNode()
        val codeText = ShownText(codeNode.textLayouts().single(), codeNode.positionInRoot)

        compose.onNodeWithContentDescription(QR_HINT).performClick()
        compose.waitForIdle()
        val qrAgain = capture()

        return RevealWalk(screen, packet, code, codeRegion, qrRegion, codeText, qrFirst = qrFirst, codeShown = codeShown, qrAgain = qrAgain)
    }

    /**
     * Before anything is revealed and after each switch, exactly one of the QR code and the code
     * must be readable, and a hidden code must carry almost nothing of what tells two codes apart
     */
    private fun assertOneValueReadableAtATime(
        what: String,
        first: RevealWalk,
        second: RevealWalk,
    ) {
        val readings = listOf(first, second).map(::readWalk)
        val codeSeparation = separation(first, second, first.codeRegion, hidden = { it.qrFirst }, shown = { it.codeShown })
        val qrSeparation = separation(first, second, first.qrRegion, hidden = { it.codeShown }, shown = { it.qrFirst })
        Log.i(TAG, "$what: $codeSeparation")
        Log.i(TAG, "QR code beside the $what: $qrSeparation")

        readings.forEach(WalkReading::assertExactlyOneReadable)

        assertEquals("the two ${what}s have different lengths", first.code.length, second.code.length)
        assertTrue("the two ${what}s are the same", first.code != second.code)
        assertEquals("the $what moved between the two transfers", first.codeRegion, second.codeRegion)
        assertTrue("the two shown ${what}s look the same: $codeSeparation", codeSeparation.shown >= MIN_SHOWN_DETAIL)
        assertTrue(
            "the hidden ${what}s differ in detail by ${"%.3f".format(codeSeparation.ratio)} of how much the shown ones do",
            codeSeparation.ratio <= MAX_HIDDEN_DETAIL_SHARE,
        )
    }

    private fun readWalk(walk: RevealWalk): WalkReading {
        val qrStates =
            listOf("before anything is revealed" to walk.qrFirst, "after showing the QR code again" to walk.qrAgain).map { (moment, image) ->
                QrState(moment, image.readQr(), image.matchCode(walk.codeText))
            }
        val reading = WalkReading(walk, qrStates, hiddenQr = walk.codeShown.readQr(), shownCode = walk.codeShown.matchCode(walk.codeText))

        qrStates.forEach { Log.i(TAG, "${walk.screen} ${it.moment}: QR code reads as ${it.qr.kind}, hidden code ${it.code}") }
        Log.i(TAG, "${walk.screen} with the code shown: hidden QR code reads as ${reading.hiddenQr.kind}, shown code ${reading.shownCode}")
        return reading
    }

    // how much two walks' captures differ over a region, while the value there is hidden and shown
    private fun separation(
        first: RevealWalk,
        second: RevealWalk,
        region: IntRect,
        hidden: (RevealWalk) -> Bitmap,
        shown: (RevealWalk) -> Bitmap,
    ) = Separation(
        hiddenMean = hidden(first).meanDifference(hidden(second), region),
        shownMean = shown(first).meanDifference(shown(second), region),
        hidden = hidden(first).detailDifference(hidden(second), region),
        shown = shown(first).detailDifference(shown(second), region),
    )

    private fun windowIsSecure(): Boolean {
        var secure = false
        compose.runOnUiThread {
            secure = compose.activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0
        }
        return secure
    }

    private fun capture(): Bitmap = compose.onRoot().captureToImage().asAndroidBitmap()

    private fun Bitmap.matchCode(text: ShownText) = matchTextReference(text.layout, text.topLeft, compose.density, "code")

    private fun Bitmap.readQr(): QrReading {
        val pixels = IntArray(width * height)
        getPixels(pixels, 0, width, 0, 0, width, height)
        val source = RGBLuminanceSource(width, height, pixels)

        return try {
            val text = QRCodeReader().decode(BinaryBitmap(HybridBinarizer(source)), mapOf(DecodeHintType.TRY_HARDER to true)).text
            QrReading(text, "a decoded QR code")
        } catch (_: NotFoundException) {
            QrReading(null, "no QR code")
        } catch (error: ReaderException) {
            QrReading(null, "a QR code zxing found but could not read (${error.javaClass.simpleName})")
        }
    }

    /** What zxing read from a capture, with [kind] safe for messages since it never holds the text */
    private class QrReading(
        val text: String?,
        val kind: String,
    )

    // the mean of each pixel's largest channel difference, over the region
    private fun Bitmap.meanDifference(
        other: Bitmap,
        region: IntRect,
    ): Double {
        var total = 0L
        for (y in region.top until region.bottom) {
            for (x in region.left until region.right) {
                val a = getPixel(x, y)
                val b = other.getPixel(x, y)
                total +=
                    maxOf(
                        abs((a shr 16 and 0xFF) - (b shr 16 and 0xFF)),
                        abs((a shr 8 and 0xFF) - (b shr 8 and 0xFF)),
                        abs((a and 0xFF) - (b and 0xFF)),
                    )
            }
        }

        return total.toDouble() / (region.width * region.height)
    }

    /**
     * How much two captures differ in fine detail over [region]: each pixel's luminance difference
     * less the mean difference around it, averaged
     *
     * Text reads from its strokes. A blur keeps only smooth brightness, which this leaves out, while
     * strokes that show through change the difference from one pixel to the next
     */
    private fun Bitmap.detailDifference(
        other: Bitmap,
        region: IntRect,
    ): Double {
        val w = region.width
        val h = region.height
        val difference =
            DoubleArray(w * h) { i ->
                val x = region.left + i % w
                val y = region.top + i / w
                luminance(getPixel(x, y)) - luminance(other.getPixel(x, y))
            }

        // summed-area table, so each pixel's neighbourhood mean costs four lookups
        val sums = DoubleArray((w + 1) * (h + 1))
        for (y in 0 until h) {
            for (x in 0 until w) {
                sums[(y + 1) * (w + 1) + x + 1] =
                    difference[y * w + x] + sums[y * (w + 1) + x + 1] + sums[(y + 1) * (w + 1) + x] - sums[y * (w + 1) + x]
            }
        }

        var total = 0.0
        for (y in 0 until h) {
            for (x in 0 until w) {
                val left = maxOf(0, x - DETAIL_RADIUS_PX)
                val top = maxOf(0, y - DETAIL_RADIUS_PX)
                val right = minOf(w, x + DETAIL_RADIUS_PX + 1)
                val bottom = minOf(h, y + DETAIL_RADIUS_PX + 1)
                val sum = sums[bottom * (w + 1) + right] - sums[top * (w + 1) + right] - sums[bottom * (w + 1) + left] + sums[top * (w + 1) + left]
                total += abs(difference[y * w + x] - sum / ((right - left) * (bottom - top)))
            }
        }

        return total / (w * h)
    }

    private fun luminance(argb: Int): Double = 0.299 * (argb shr 16 and 0xFF) + 0.587 * (argb shr 8 and 0xFF) + 0.114 * (argb and 0xFF)

    private class ShownText(
        val layout: TextLayoutResult,
        val topLeft: Offset,
    )

    private fun SemanticsNodeInteraction.regionInRoot(): IntRect = fetchSemanticsNode().boundsInRoot.roundToIntRect()

    private class RevealWalk(
        val screen: String,
        val packet: String,
        val code: String,
        val codeRegion: IntRect,
        val qrRegion: IntRect,
        val codeText: ShownText,
        val qrFirst: Bitmap,
        val codeShown: Bitmap,
        val qrAgain: Bitmap,
    )

    private class QrState(
        val moment: String,
        val qr: QrReading,
        val code: TextReferenceMatch,
    )

    private class WalkReading(
        val walk: RevealWalk,
        val qrStates: List<QrState>,
        val hiddenQr: QrReading,
        val shownCode: TextReferenceMatch,
    ) {
        fun assertExactlyOneReadable() {
            val screen = walk.screen
            qrStates.forEach {
                assertTrue("the QR code of $screen reads as ${it.qr.kind}, not its packet, ${it.moment}", it.qr.text == walk.packet)
                assertTrue("the hidden code of $screen matches its own text ${it.moment}: ${it.code}", !it.code.matches)
            }

            assertTrue("the hidden QR code of $screen reads as ${hiddenQr.kind} while the code is shown", hiddenQr.text == null)
            assertTrue("the shown code of $screen is not drawn as its own text: $shownCode", shownCode.matches)
        }
    }

    /** Mean and fine-detail differences between two walks' captures of one region, hidden and shown */
    private class Separation(
        val hiddenMean: Double,
        val shownMean: Double,
        val hidden: Double,
        val shown: Double,
    ) {
        val ratio: Double get() = hidden / shown

        override fun toString(): String =
            "detail difference hidden ${"%.3f".format(hidden)} / shown ${"%.3f".format(shown)} = ${"%.4f".format(ratio)}, " +
                "mean difference hidden ${"%.3f".format(hiddenMean)} / shown ${"%.3f".format(shownMean)} = ${"%.4f".format(hiddenMean / shownMean)}"
    }

    private companion object {
        const val TAG = "KeyTeleportRevealCapture"
        const val QR_HINT = "Tap to show QR code"
        const val RECEIVER_CODE_HINT = "Tap to show receiver code"
        const val SENDER_PASSWORD_HINT = "Tap to show password"

        // TODO measured
        const val MIN_SHOWN_DETAIL = 1.0
        const val MAX_HIDDEN_DETAIL_SHARE = 0.2

        // about a glyph stroke at the code's size, so the neighbourhood mean keeps a blur's smooth shading
        const val DETAIL_RADIUS_PX = 4

        // published BIP39 test vector, never used for funds; distinct from the vectors other tests import
        val TEST_WORDS =
            listOf(
                "cat", "swing", "flag", "economy", "stadium", "alone",
                "churn", "speed", "unique", "patch", "report", "train",
            )
        val TEST_WALLET = TestWalletWords(TEST_WORDS, masterFingerprint = "dd1fad4d")
    }
}
