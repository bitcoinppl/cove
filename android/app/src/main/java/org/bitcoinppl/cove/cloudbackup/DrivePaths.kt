package org.bitcoinppl.cove.cloudbackup

import java.security.MessageDigest
import org.bitcoinppl.cove_core.csppMasterKeyRecordId
import org.bitcoinppl.cove_core.csppNamespacesSubdirectory
import org.bitcoinppl.cove_core.csppWalletFilePrefix

internal object DrivePaths {
    private const val MASTER_KEY_FILE_PREFIX = "masterkey-"

    val namespacesRootFolderName: String = csppNamespacesSubdirectory()

    val masterKeyFileName: String =
        buildString {
            append(MASTER_KEY_FILE_PREFIX)
            append(sha256Hex(csppMasterKeyRecordId()))
            append(".json")
        }

    fun walletFileName(recordId: String): String = "${csppWalletFilePrefix()}$recordId.json"

    fun isWalletFile(name: String): Boolean =
        name.startsWith(csppWalletFilePrefix()) && name.endsWith(".json")

    private fun sha256Hex(input: String): String =
        MessageDigest
            .getInstance("SHA-256")
            .digest(input.toByteArray())
            .joinToString(separator = "") { byte -> "%02x".format(byte) }
}
