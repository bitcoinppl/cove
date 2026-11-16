package org.bitcoinppl.cove.cloudbackup

import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupSection
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

class CloudBackupVerificationSectionTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun retryActionsAreLaidOutBelowErrorMessage() {
        val error =
            "cloud storage error: failed to download master key backup: " +
                "authorization required: google drive authorization was cancelled"
        val screen =
            cloudBackupFixture("configured-passkey-missing").withVerification(
                CloudBackupVerificationState.Failed(
                    DeepVerificationFailure.Retry(
                        message = error,
                        detail = null,
                        retryAction = null,
                    ),
                ),
            )

        compose.setVerificationSection(screen)

        val errorBottom = compose.onNodeWithText(error).getUnclippedBoundsInRoot().bottom
        val retryTop = compose.onNodeWithText("Try Again").getUnclippedBoundsInRoot().top

        assertTrue("retry action should appear below the error", retryTop >= errorBottom)

        // a failure off the passkey never offers a passkey repair; only the passkey card does
        compose.onAllNodesWithText("Create New Passkey").assertCountEquals(0)
    }

    @Test
    fun undecryptableIssueRequiresDeletionConfirmation() {
        val issue = "2 wallet backup(s) could not be decrypted with this backup key"
        val screen = cloudBackupFixture("configured-needs-attention-undecryptable")

        compose.setVerificationSection(screen)

        compose.onNodeWithText(issue).performClick()

        compose.onNodeWithText("Delete 2 Inaccessible Wallet Backups?").assertIsDisplayed()
        compose.onNodeWithText("Delete Backups").assertIsDisplayed()
        compose.onNodeWithText("Cancel").assertIsDisplayed()
    }

    @Test
    fun notCompletedRequestIsANeutralResultWithoutPasskeyRepair() {
        // Rust offers the repair here, since the passkey is missing, but a cancelled request
        // shows nothing about the passkey, so the repair stays on the passkey card
        val screen =
            cloudBackupFixture("configured-passkey-missing")
                .withAvailable(CloudBackupScreenAction.REPAIR_PASSKEY)
                .withVerification(CloudBackupVerificationState.NotCompleted)

        compose.setVerificationSection(screen)

        compose.onNodeWithText("Cloud Backup Not Verified").assertIsDisplayed()
        compose
            .onNodeWithText("Verification didn't finish. Verify again to confirm your cloud backup can be restored.")
            .assertIsDisplayed()
        compose.onNodeWithText("Verify Now").assertIsDisplayed()
        compose.onAllNodesWithText("Add New Passkey").assertCountEquals(0)
        compose.onAllNodesWithText("If your passkey was deleted", substring = true).assertCountEquals(0)
    }

    @Test
    fun notCompletedRequestBannerNamesThePasswordManagerToUnlock() {
        // a locked 1Password ends the request like a cancel, so the banner at the top says to
        // unlock it before verifying again
        val screen = cloudBackupFixture("configured-verification-not-completed")
        val configured = (screen.body as CloudBackupScreenBody.Configured).v1
        val banner = configured.sections.first() as CloudBackupSection.PasskeyUnlock

        compose.setContent {
            CoveTheme(darkTheme = false) {
                PasskeyUnlockBanner(banner.v1)
            }
        }

        compose.onNodeWithText("Unlock 1Password").assertIsDisplayed()
        compose
            .onNodeWithText(
                "Verification couldn't use your passkey. If 1Password is locked, unlock it, then tap Verify Now.",
            ).assertIsDisplayed()
    }

    @Test
    fun notCompletedSectionRepeatsTheUnlockBannerAboveItself() {
        // the section can sit far below the top banner, so it draws the same banner above itself
        val screen = cloudBackupFixture("configured-verification-not-completed")

        compose.setVerificationSection(screen)

        val bannerBottom = compose.onNodeWithText("Unlock 1Password").getUnclippedBoundsInRoot().bottom
        val sectionTop = compose.onNodeWithText("Cloud Backup Not Verified").getUnclippedBoundsInRoot().top

        assertTrue("the unlock banner should appear above the section", bannerBottom <= sectionTop)
        compose.onNodeWithText("Verify Now").assertIsDisplayed()
    }

    private fun ComposeContentTestRule.setVerificationSection(screen: CloudBackupScreen) {
        setContent {
            CoveTheme(darkTheme = false) {
                VerificationSection(
                    manager = CloudBackupManager(screen),
                    screen = screen,
                    section = screen.requireVerificationSection(),
                    onRecreate = {},
                    onReinitialize = {},
                )
            }
        }
    }
}
