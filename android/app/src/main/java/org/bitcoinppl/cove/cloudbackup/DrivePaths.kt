package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.csppMasterKeyDirectory
import org.bitcoinppl.cove_core.csppNamespacesSubdirectory
import org.bitcoinppl.cove_core.csppWalletFilePrefix
import org.bitcoinppl.cove_core.csppWalletsDirectory

internal object DrivePaths {
    val defaultNames: DrivePathNames by lazy {
        DrivePathNames(
            namespacesRootFolderName = csppNamespacesSubdirectory(),
            masterKeyFolderName = csppMasterKeyDirectory(),
            walletsFolderName = csppWalletsDirectory(),
            walletFilePrefix = csppWalletFilePrefix(),
        )
    }
}

internal data class DrivePathNames(
    val namespacesRootFolderName: String,
    val masterKeyFolderName: String,
    val walletsFolderName: String,
    val walletFilePrefix: String,
) {
    fun walletLocationForFileName(fileName: String): String = "$walletsFolderName/$fileName"

    fun isWalletFile(name: String): Boolean =
        isWalletFileLocation(
            location = name,
            walletFilePrefix = walletFilePrefix,
            walletsFolderName = walletsFolderName,
        )
}

internal fun isWalletFileLocation(
    location: String,
    walletFilePrefix: String,
    walletsFolderName: String,
): Boolean =
    location
        .removePrefix("$walletsFolderName/")
        .takeUnless { it.contains("/") }
        ?.let { fileName -> fileName.startsWith(walletFilePrefix) && fileName.endsWith(".json") }
        ?: false
