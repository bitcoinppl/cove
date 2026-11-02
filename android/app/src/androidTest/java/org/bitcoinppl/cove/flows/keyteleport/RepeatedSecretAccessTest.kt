package org.bitcoinppl.cove.flows.keyteleport

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.bitcoinppl.cove.ImportWalletManager
import org.bitcoinppl.cove.test.TestWalletWords
import org.bitcoinppl.cove.test.awaitOnMain
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.deleteTestWallet
import org.bitcoinppl.cove.test.importTestWalletOrNull
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.ImportWalletException
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.bitcoinppl.cove_core.Mnemonic
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Reading a hot wallet's recovery words over and over, writing the same wallet again, and running
 * several KeyTeleport sends of it must keep exactly one wallet whose words read back unchanged
 *
 * Runs against the real Android keystore through the Rust keychain, so a read that fails, hangs or
 * returns different words fails here. Each send is received on this device too, where the received
 * words must match and importing them must find the existing wallet instead of adding a copy
 */
@RunWith(AndroidJUnit4::class)
class RepeatedSecretAccessTest {
    private lateinit var walletId: WalletId

    @Before
    fun setUp() {
        bootstrapRustRuntimeForUiTest()

        // the test deletes its wallet afterwards, so never run it over one it did not create
        val imported = importTestWalletOrNull(TEST_WALLET)
        assumeTrue("the test wallet already exists on this device", imported != null)
        walletId = requireNotNull(imported).id
    }

    @After
    fun tearDown() {
        if (::walletId.isInitialized) deleteTestWallet(walletId)
    }

    @Test
    fun repeatedReadsWritesAndKeyTeleportSendsKeepOneReadableWallet() {
        val walletsBefore = walletIds()
        assertEquals("the test wallet is not listed exactly once", 1, walletsBefore.count { it == walletId })

        repeat(READ_ROUNDS) { round ->
            assertEquals("read $round returned different recovery words", TEST_WORDS, readWords())
        }

        repeat(WRITE_ROUNDS) { round ->
            assertEquals("write $round did not find the existing wallet", walletId, writeWordsAgain())
        }

        repeat(SEND_ROUNDS) { round ->
            sendToThisDeviceAndImport(round)
        }

        assertEquals("the wallet list changed", walletsBefore, walletIds())
        assertEquals("the recovery words changed after the sends", TEST_WORDS, readWords())
    }

    private fun sendToThisDeviceAndImport(round: Int) {
        KeyTeleportTestPeer().use { receiver ->
            val request = receiver.startReceive()
            val response = KeyTeleportTestPeer().use { sender -> sender.sendWallet(walletId, request) }
            val accepted =
                awaitOnMain("send $round to reach the receiver") {
                    receiver.manager.ingestKeyTeleportText(response.packetText, KeyTeleportFlowDirection.RECEIVE)
                }

            assertTrue("send $round response was rejected", accepted)
            awaitState(receiver, "send $round to ask for the password") {
                it is KeyTeleportManagerState.ReceiveEnterPassword
            }

            receiver.manager.dispatch(KeyTeleportManagerAction.EnterSenderPassword(response.password))
            awaitState(receiver, "send $round to unlock") { it is KeyTeleportManagerState.ReceiveMnemonicReview }
            val received = awaitOnMain("send $round words") { receiver.manager.secrets.revealMnemonicWords() }
            assertEquals("send $round delivered different recovery words", TEST_WORDS, received)

            receiver.manager.dispatch(KeyTeleportManagerAction.ImportReceivedWallet)
            val existing =
                awaitOnMain("send $round import to finish") {
                    (receiver.manager.state as? KeyTeleportManagerState.ReceiveAlreadyImportedWallet)?.v1?.id
                }

            assertEquals("send $round matched a different wallet", walletId, existing)
        }
    }

    private fun awaitState(
        peer: KeyTeleportTestPeer,
        what: String,
        matches: (KeyTeleportManagerState) -> Boolean,
    ) {
        awaitOnMain(what) { peer.manager.state.takeIf(matches) }
    }

    // a hot wallet that already holds these words reports itself instead of being written twice
    private fun writeWordsAgain(): WalletId? =
        try {
            ImportWalletManager().use { manager -> manager.importWallet(listOf(TEST_WORDS)) }
            null
        } catch (error: ImportWalletException.WalletAlreadyExists) {
            error.v1
        }

    private fun readWords(): List<String> = Mnemonic(id = walletId).use { it.words() }

    private fun walletIds(): List<WalletId> = Database().wallets().all().map { it.id }

    private companion object {
        const val READ_ROUNDS = 25
        const val WRITE_ROUNDS = 3
        const val SEND_ROUNDS = 4

        // published BIP39 test vector, never used for funds; distinct from the vectors other tests import
        val TEST_WORDS =
            listOf(
                "zoo", "zoo", "zoo", "zoo", "zoo", "zoo",
                "zoo", "zoo", "zoo", "zoo", "zoo", "wrong",
            )
        val TEST_WALLET = TestWalletWords(TEST_WORDS, masterFingerprint = "3f635a63")
    }
}
