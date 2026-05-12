package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.csppMasterKeyDirectory
import org.bitcoinppl.cove_core.csppNamespacesSubdirectory
import org.bitcoinppl.cove_core.csppWalletFilePrefix
import org.bitcoinppl.cove_core.csppWalletsDirectory

internal object DrivePaths {
    val namespacesRootFolderName: String = csppNamespacesSubdirectory()
    val masterKeyFolderName: String = csppMasterKeyDirectory()
    val walletsFolderName: String = csppWalletsDirectory()

    fun walletLocationForFileName(fileName: String): String = "$walletsFolderName/$fileName"

    fun isWalletFile(name: String): Boolean =
        isWalletFileLocation(
            location = name,
            walletFilePrefix = csppWalletFilePrefix(),
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
