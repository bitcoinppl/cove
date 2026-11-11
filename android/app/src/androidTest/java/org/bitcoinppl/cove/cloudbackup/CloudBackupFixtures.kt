package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupActionEntry
import org.bitcoinppl.cove_core.CloudBackupActions
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupScreenFixture
import org.bitcoinppl.cove_core.CloudBackupSection
import org.bitcoinppl.cove_core.CloudBackupVerificationSection
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.cloudBackupScreenFixtures

// Rust prefixes every fixture name so the release-seam check can find it; tests use the short name
private const val FIXTURE_NAME_PREFIX = "cloud-backup-screen-fixture:"

internal val CloudBackupScreenFixture.shortName: String
    get() = name.removePrefix(FIXTURE_NAME_PREFIX)

internal fun cloudBackupFixture(name: String): CloudBackupScreen =
    cloudBackupScreenFixtures().firstOrNull { it.shortName == name }?.screen
        ?: error("no Cloud Backup screen fixture named $name")

// adds actions a fixture leaves out, so one test can reach a control without a new Rust fixture
internal fun CloudBackupScreen.withAvailable(vararg actions: CloudBackupScreenAction): CloudBackupScreen =
    copy(
        actions =
            CloudBackupActions(
                this.actions.entries +
                    actions.map { CloudBackupActionEntry(it, CloudBackupActionAvailability.Available) },
            ),
    )

// swaps the verification state of a configured fixture, keeping its verification controls and
// the rest of its sections
internal fun CloudBackupScreen.withVerification(state: CloudBackupVerificationState): CloudBackupScreen =
    mapSections { section ->
        when (section) {
            is CloudBackupSection.Verification ->
                CloudBackupSection.Verification(section.v1.copy(view = section.v1.view.copy(state = state)))
            else -> section
        }
    }

internal fun CloudBackupScreen.requireVerificationSection(): CloudBackupVerificationSection =
    configured
        ?.sections
        ?.firstNotNullOfOrNull { (it as? CloudBackupSection.Verification)?.v1 }
        ?: error("fixture has no verification section")

// swaps the Restore All state of a configured fixture's Not on This Device section
internal fun CloudBackupScreen.withRestoreAll(state: CloudBackupRestoreAllState): CloudBackupScreen =
    mapSections { section ->
        when (section) {
            is CloudBackupSection.NotOnDevice -> CloudBackupSection.NotOnDevice(section.v1.copy(restoreAll = state))
            else -> section
        }
    }

private fun CloudBackupScreen.mapSections(transform: (CloudBackupSection) -> CloudBackupSection): CloudBackupScreen {
    val body = body as? CloudBackupScreenBody.Configured ?: error("fixture is not configured")

    return copy(body = CloudBackupScreenBody.Configured(body.v1.copy(sections = body.v1.sections.map(transform))))
}
