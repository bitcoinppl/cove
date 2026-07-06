package org.bitcoinppl.cove.cloudbackup

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class DriveAccountIdentityTest {
    @Test
    fun driveAccountIdentityNormalizesEmailForEqualityAndHashCode() {
        val first = DriveAccountIdentity(googleAccountId = "account-1", email = " Person@Example.com ")
        val second = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")

        assertEquals(first, second)
        assertEquals(first.hashCode(), second.hashCode())
        assertEquals("person@example.com", first.email)
    }

    @Test
    fun driveAccountIdentityKeepsIdSourcesSeparate() {
        val googleIdentity = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")
        val driveIdentity = DriveAccountIdentity(drivePermissionId = "permission-1", email = "PERSON@example.com")

        assertTrue(googleIdentity.matches(driveIdentity))
        assertEquals(
            DriveAccountIdentity(
                googleAccountId = "account-1",
                drivePermissionId = "permission-1",
                email = "person@example.com",
            ),
            googleIdentity.verifiedMerge(driveIdentity),
        )
    }

    @Test
    fun driveAccountIdentityMergePoliciesHandleEmailRefreshes() {
        val original = DriveAccountIdentity(googleAccountId = "account-1", email = "old@example.com")
        val refreshed = DriveAccountIdentity(
            googleAccountId = "account-1",
            drivePermissionId = "permission-1",
            email = "new@example.com",
        )

        assertEquals(
            DriveAccountIdentity(
                googleAccountId = "account-1",
                drivePermissionId = "permission-1",
                email = "old@example.com",
            ),
            original.withMissingFieldsFrom(refreshed),
        )
        assertEquals(
            DriveAccountIdentity(
                googleAccountId = "account-1",
                drivePermissionId = "permission-1",
                email = "new@example.com",
            ),
            original.verifiedMerge(refreshed),
        )
    }

    @Test
    fun driveAccountIdentityMergePoliciesFallbackWithoutRefreshedEmail() {
        val original = DriveAccountIdentity(googleAccountId = "account-1", email = "old@example.com")
        val withoutEmail = DriveAccountIdentity(
            googleAccountId = "account-1",
            drivePermissionId = "permission-1",
            email = null,
        )
        val expected = DriveAccountIdentity(
            googleAccountId = "account-1",
            drivePermissionId = "permission-1",
            email = "old@example.com",
        )

        assertEquals(expected, original.withMissingFieldsFrom(withoutEmail))
        assertEquals(expected, original.verifiedMerge(withoutEmail))
    }

    @Test
    fun driveAccountIdentityWithMissingFieldsDoesNotOverwriteExistingIds() {
        val original = DriveAccountIdentity(
            googleAccountId = "account-1",
            drivePermissionId = "permission-1",
            email = "person@example.com",
        )
        val other = DriveAccountIdentity(
            googleAccountId = "account-2",
            drivePermissionId = "permission-2",
            email = "other@example.com",
        )

        assertEquals(original, original.withMissingFieldsFrom(other))
    }
}
