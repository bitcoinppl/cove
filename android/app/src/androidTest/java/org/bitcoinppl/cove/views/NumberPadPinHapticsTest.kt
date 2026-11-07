package org.bitcoinppl.cove.views

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import java.util.Collections

/**
 * The PIN pad gives one key press haptic for each digit or delete it accepts and none for a tap it
 * ignores, and submits each PIN once with the digits typed
 *
 * The pad's haptic goes through `LocalHapticFeedback`, so a recorder stands in for the device's
 * vibrator here. How strong the haptic feels is left to a person
 */
class NumberPadPinHapticsTest {
    @get:Rule
    val compose = createComposeRule()

    private val haptics: MutableList<HapticFeedbackType> = Collections.synchronizedList(mutableListOf())
    private val checked: MutableList<String> = Collections.synchronizedList(mutableListOf())

    /** Holds each PIN check open until the test answers it, so taps can land while it runs */
    private var answer = CompletableDeferred<Boolean>()

    @Test
    fun acceptedTapsGiveOneHapticAndEachPinSubmitsOnce() {
        compose.setContent {
            CompositionLocalProvider(LocalHapticFeedback provides RecordingHapticFeedback(haptics)) {
                NumberPadPinView(
                    title = "Enter Cove PIN",
                    isPinCorrect = { pin ->
                        checked += pin
                        answer.await()
                    },
                )
            }
        }

        // delete on an empty pad does nothing, so it gives no haptic
        tapDelete()
        assertHaptics(0, "a delete on an empty pad")

        // a delete takes the last digit back, so 1 2 3 4 delete 5 6 7 is 1 2 3 5 6 7
        "1234".forEach(::tapDigit)
        tapDelete()
        assertHaptics(5, "four digits and a delete")

        // the clock stops so taps land inside the pad's 100 ms wait before it checks the PIN
        compose.mainClock.autoAdvance = false
        "567".forEach(::tapDigit)
        assertHaptics(8, "the last three digits")

        tapDigit('9')
        tapDelete()
        assertHaptics(8, "taps in the 100 ms before the PIN check")
        assertEquals("the PIN was checked before the pad's 100 ms wait ended", emptyList<String>(), checked.toList())

        compose.mainClock.advanceTimeBy(150)
        compose.waitUntil(timeoutMillis = 5_000) { checked.isNotEmpty() }
        assertEquals("the pad should check the PIN typed, once", listOf("123567"), checked.toList())

        // taps while the check runs are ignored too, and can't start a second check
        tapDigit('8')
        tapDelete()
        tapDigit('0')
        assertHaptics(8, "taps while the PIN check runs")

        // a wrong PIN shakes the field and clears it, then the pad takes a new PIN
        compose.mainClock.autoAdvance = true
        val first = answer
        answer = CompletableDeferred()
        first.complete(false)
        compose.waitForIdle()

        tapDelete()
        assertHaptics(8, "a delete on the pad a wrong PIN cleared")

        "246802".forEach(::tapDigit)
        assertHaptics(14, "a second PIN")
        compose.waitUntil(timeoutMillis = 5_000) { checked.size == 2 }
        answer.complete(true)
        compose.waitForIdle()

        assertEquals("each PIN should be checked once, as typed", listOf("123567", "246802"), checked.toList())
        assertEquals("every haptic should be a key press", listOf(HapticFeedbackType.KeyboardTap), haptics.distinct())
    }

    private fun tapDigit(digit: Char) {
        compose.onNodeWithTag("numberPadPin.digit.$digit").performClick()
    }

    private fun tapDelete() {
        compose.onNodeWithTag("numberPadPin.delete").performClick()
    }

    private fun assertHaptics(
        expected: Int,
        after: String,
    ) {
        assertEquals("haptics after $after", expected, haptics.size)
    }
}

/** Records each haptic the pad asks for instead of vibrating */
private class RecordingHapticFeedback(
    private val performed: MutableList<HapticFeedbackType>,
) : HapticFeedback {
    override fun performHapticFeedback(hapticFeedbackType: HapticFeedbackType) {
        performed += hapticFeedbackType
    }
}
