package org.bitcoinppl.cove.cloudbackup

import java.net.HttpURLConnection
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DriveFolderResolverTest {
    @Test
    fun driveLocationPartsKeepsFlatFilesAtNamespaceRoot() {
        assertEquals(
            DriveLocationParts(parentFolders = emptyList(), fileName = "wallet-record.json"),
            driveLocationParts("wallet-record.json"),
        )
    }

    @Test
    fun driveLocationPartsSplitsKindPrefixedFiles() {
        assertEquals(
            DriveLocationParts(parentFolders = listOf("wallets"), fileName = "wallet-record.json"),
            driveLocationParts("wallets/wallet-record.json"),
        )
    }

    @Test
    fun driveLocationPartsRejectsParentTraversal() {
        val error = runCatching { driveLocationParts("wallets/../wallet-record.json") }
            .exceptionOrNull()

        assertTrue(error is IllegalArgumentException)
    }

    @Test
    fun driveLocationPartsRejectsBlankRelativePath() {
        val error = runCatching { driveLocationParts("") }
            .exceptionOrNull()

        assertTrue(error is IllegalArgumentException)
        assertEquals("relativePath must not be blank", error?.message)
    }

    @Test
    fun drivePathsAcceptLegacyFlatAndKindPrefixedWalletLocations() {
        assertTrue(
            isWalletFileLocation(
                location = "wallet-record.json",
                walletFilePrefix = "wallet-",
                walletsFolderName = "wallets",
            ),
        )
        assertTrue(
            isWalletFileLocation(
                location = "wallets/wallet-record.json",
                walletFilePrefix = "wallet-",
                walletsFolderName = "wallets",
            ),
        )
        assertFalse(
            isWalletFileLocation(
                location = "master-key/wallet-record.json",
                walletFilePrefix = "wallet-",
                walletsFolderName = "wallets",
            ),
        )
    }

    @Test
    fun duplicateDriveFolderNamesAreDetected() {
        assertEquals(
            setOf("wallets"),
            duplicateDriveFolderNames(listOf("master-key", "wallets", "wallets")),
        )
        assertTrue(duplicateDriveFolderNames(listOf("master-key", "wallets")).isEmpty())
    }

    @Test
    fun duplicateDriveFileNamesAreDetected() {
        assertEquals(
            setOf("master-key.json"),
            duplicateDriveFileNames(listOf("master-key.json", "wallet-record.json", "master-key.json")),
        )
        assertTrue(duplicateDriveFileNames(listOf("master-key.json", "wallet-record.json")).isEmpty())
    }

    @Test
    fun backupFileLocationsRejectDuplicateJsonFiles() {
        val error =
            runCatching {
                driveBackupFileLocations(
                    listOf("master-key.json", "notes.txt", "master-key.json"),
                )
            }.exceptionOrNull()

        assertTrue(error is DriveHttpException)
        assertEquals(HttpURLConnection.HTTP_CONFLICT, (error as DriveHttpException).statusCode)
        assertEquals("duplicate google drive file: master-key.json", error.body)
    }

    @Test
    fun backupFileLocationsIgnoreNonJsonFilesAndApplyLocation() {
        assertEquals(
            listOf("wallets/wallet-record.json"),
            driveBackupFileLocations(
                listOf("wallet-record.json", "notes.txt"),
                { fileName -> "wallets/$fileName" },
            ),
        )
    }

    @Test
    fun cloudBackupNamespaceValidationMatchesRustShape() {
        assertTrue(isValidCloudBackupNamespaceId("0123456789abcdef0123456789abcdef"))
        assertFalse(isValidCloudBackupNamespaceId("0123456789ABCDEF0123456789abcdef"))
        assertFalse(isValidCloudBackupNamespaceId("../0123456789abcdef0123456789abcd"))
        assertFalse(isValidCloudBackupNamespaceId("0123456789abcdef"))
    }
}
