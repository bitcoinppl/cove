package org.bitcoinppl.cove.cloudbackup

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class DriveAccountBindingTest {
    @Test
    fun driveAccountBindingPersistsFirstAccountAndRejectsMismatch() {
        val store = TestDriveAccountBindingStore()
        val first = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")
        val sameEmailFallback = DriveAccountIdentity(googleAccountId = null, email = "PERSON@example.com")
        val mismatch = DriveAccountIdentity(googleAccountId = "account-2", email = "other@example.com")

        verifyDriveAccountBinding(store, first)
        verifyDriveAccountBinding(store, sameEmailFallback)

        val error = runCatching { verifyDriveAccountBinding(store, mismatch) }.exceptionOrNull()

        assertTrue(error is DriveAccountBindingException.Mismatch)
    }

    @Test
    fun driveAccountBindingMatchesStoredGoogleIdToDrivePermissionIdByEmail() {
        val store = TestDriveAccountBindingStore(
            DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com"),
        )
        val fallback = DriveAccountIdentity(drivePermissionId = "permission-1", email = "PERSON@example.com")

        verifyDriveAccountBinding(store, fallback)

        assertEquals(
            DriveAccountIdentity(
                googleAccountId = "account-1",
                drivePermissionId = "permission-1",
                email = "person@example.com",
            ),
            store.selectedIdentity(),
        )
    }

    @Test
    fun driveAccountBindingValidationDoesNotPersistUnverifiedAccount() {
        val store = TestDriveAccountBindingStore()
        val probe = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")

        verifyDriveAccountBinding(store, probe, bindIfMissing = false)

        assertEquals(null, store.selectedIdentity())
    }

    @Test
    fun driveAccountBindingCanBeClearedAndRebound() {
        val store = TestDriveAccountBindingStore()
        val first = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")
        val second = DriveAccountIdentity(googleAccountId = "account-2", email = "other@example.com")

        verifyDriveAccountBinding(store, first)
        store.clearIdentity()
        verifyDriveAccountBinding(store, second)

        assertEquals(second, store.selectedIdentity())
    }

    @Test
    fun driveAccountBindingRejectsMissingIdentityWhenNoAccountIsSelected() {
        val store = TestDriveAccountBindingStore()

        val error = runCatching { verifyDriveAccountBinding(store, identity = null) }
            .exceptionOrNull()

        assertTrue(error is DriveAccountBindingException.MissingIdentity)
    }

    @Test
    fun driveAccountBindingRejectsMissingIdentityWhenAccountIsSelected() {
        val store = TestDriveAccountBindingStore(
            DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com"),
        )

        val error = runCatching { verifyDriveAccountBinding(store, identity = null) }
            .exceptionOrNull()

        assertTrue(error is DriveAccountBindingException.MissingIdentity)
    }

    @Test
    fun driveAccountBindingRejectsMissingIdentityWhenSelectedAccountCannotConstrainAuthorization() {
        val store = TestDriveAccountBindingStore(
            DriveAccountIdentity(googleAccountId = "account-1", email = null),
        )

        val error = runCatching { verifyDriveAccountBinding(store, identity = null) }
            .exceptionOrNull()

        assertTrue(error is DriveAccountBindingException.MissingIdentity)
    }

    @Test
    fun driveAccountBindingEnrichesSparseSelectedAccountAfterMatchingVerification() {
        val store = TestDriveAccountBindingStore(
            DriveAccountIdentity(googleAccountId = "account-1", email = null),
        )
        val verified = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")

        verifyDriveAccountBinding(store, verified)

        assertEquals(verified, store.selectedIdentity())
    }

    @Test
    fun driveAccountBindingEnrichmentPreservesCommittedTransitionReplay() {
        val store = TestDriveAccountBindingStore()
        val staged = DriveAccountIdentity(googleAccountId = "account-1", email = null)
        val verified = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")

        assertEquals(DriveAccountTransitionResult.Applied, store.stageIdentity(7UL, staged))
        assertEquals(DriveAccountTransitionResult.Applied, store.commitStagedIdentity(7UL))

        verifyDriveAccountBinding(store, verified)

        assertEquals(verified, store.selectedIdentity())
        assertEquals(DriveAccountTransitionResult.Applied, store.commitStagedIdentity(7UL))
        assertEquals(DriveAccountTransitionResult.Applied, store.finalizeCommittedIdentity(7UL))
        assertEquals(DriveAccountTransitionResult.WrongTransition, store.commitStagedIdentity(7UL))
    }

    @Test
    fun driveAccountBindingRejectsWrongTransitionWithoutChangingStagedIdentity() {
        val original = DriveAccountIdentity(googleAccountId = "account-1")
        val replacement = DriveAccountIdentity(googleAccountId = "account-2")
        val store = TestDriveAccountBindingStore(original)

        assertEquals(DriveAccountTransitionResult.Applied, store.stageIdentity(7UL, replacement))

        assertEquals(DriveAccountTransitionResult.WrongTransition, store.commitStagedIdentity(8UL))
        assertEquals(DriveAccountTransitionResult.WrongTransition, store.rollbackStagedIdentity(8UL))
        assertEquals(
            DriveAccountBindingState.Staged(7UL, original, replacement),
            store.state(),
        )
    }
}
