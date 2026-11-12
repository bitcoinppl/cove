@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.isToggleable
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.GlobalConfigKey
import org.bitcoinppl.cove_core.PayjoinSetting
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class PayjoinSettingTest {
    @get:Rule
    val compose = createComposeRule()

    @Before
    fun bootstrap() {
        bootstrapRustRuntimeForUiTest()
        forgetChoice()
    }

    @After
    fun forgetChoice() {
        Database().globalConfig().delete(GlobalConfigKey.PayjoinChoice)
    }

    // the build's rollout decides the default, so the test starts from whatever Rust reports
    @Test
    fun theSwitchShowsTheRolloutDefaultAndSavesTheUsersChoice() {
        val initial = Database().globalConfig().payjoinSetting()
        compose.setContent { CoveTheme { PayjoinSection() } }

        if (initial !is PayjoinSetting.Available) {
            compose.onNodeWithText("Payjoin").assertDoesNotExist()
            return
        }

        val switch = compose.onNode(isToggleable())
        if (initial.enabled) switch.assertIsOn() else switch.assertIsOff()

        switch.performClick()
        compose.waitForIdle()

        assertEquals(
            PayjoinSetting.Available(enabled = !initial.enabled, beta = initial.beta),
            Database().globalConfig().payjoinSetting(),
        )
        if (initial.enabled) switch.assertIsOff() else switch.assertIsOn()

        switch.performClick()
        compose.waitForIdle()

        assertEquals(
            PayjoinSetting.Available(enabled = initial.enabled, beta = initial.beta),
            Database().globalConfig().payjoinSetting(),
        )
    }
}
