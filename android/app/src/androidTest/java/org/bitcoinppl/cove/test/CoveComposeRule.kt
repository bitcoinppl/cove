package org.bitcoinppl.cove.test

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.rules.ActivityScenarioRule
import kotlinx.coroutines.test.StandardTestDispatcher

// Every Compose rule in the instrumented tests comes from here, and
// .github/scripts/ci_compose_rules.py fails CI and pre-push on a rule made anywhere else
//
// Without a TestDispatcher in effectContext, Compose's test environment runs composition, effects
// and its test frame clock on an UnconfinedTestDispatcher. An effect that resumes after
// withContext(Dispatchers.IO), or from a Rust callback thread, then runs its continuation inline on
// that thread, and the frame it schedules calls measureAndLayoutForTest there while the UI thread
// may be laying out, which fails with "performMeasureAndLayout called during measure layout" or
// leaves the screen without the state the effect set. A StandardTestDispatcher queues those resumes
// on the test scheduler, which Compose advances on the UI thread whenever the test waits for idle

/**
 * A Compose rule hosting its content in a blank [ComponentActivity], whose effects resume on the UI
 * thread
 *
 * Use it instead of `createComposeRule()`; see the note at the top of this file for why
 */
fun coveComposeRule(): ComposeContentTestRule = createComposeRule(effectContext = StandardTestDispatcher())

/**
 * A Compose rule that launches activity [A], whose effects resume on the UI thread
 *
 * Use it instead of `createAndroidComposeRule<A>()`; see the note at the top of this file for why
 */
inline fun <reified A : ComponentActivity> coveAndroidComposeRule(): AndroidComposeTestRule<ActivityScenarioRule<A>, A> =
    createAndroidComposeRule<A>(effectContext = StandardTestDispatcher())
