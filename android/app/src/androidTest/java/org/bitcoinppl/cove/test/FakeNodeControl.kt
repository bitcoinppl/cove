package org.bitcoinppl.cove.test

import org.bitcoinppl.cove_core.WalletAddressType
import org.bitcoinppl.cove_core.types.Network
import org.json.JSONArray
import org.json.JSONObject

/** Esplora URL of the fake node the runner forwards into the device with `adb reverse` */
const val FAKE_NODE_ESPLORA_URL = "http://127.0.0.1:39001"

/** An output on the fake node's chain */
data class FakeNodeOutPoint(
    val txid: String,
    val vout: Int,
) {
    override fun toString(): String = "$txid:$vout"

    companion object {
        fun parse(value: String): FakeNodeOutPoint {
            val (txid, vout) = value.split(":", limit = 2)

            return FakeNodeOutPoint(txid, vout.toInt())
        }
    }
}

/** A funding output returned by `/fund` and `/fund-wallet` */
data class FakeNodeFundedOutput(
    val outpoint: FakeNodeOutPoint,
    val sats: Long,
    val address: String,
)

/** A transaction the app broadcast, as logged by `/broadcasts` */
data class FakeNodeBroadcast(
    val txid: String,
    val inputs: List<FakeNodeOutPoint>,
    val outputs: List<FakeNodeTxOut>,
)

data class FakeNodeTxOut(
    val address: String?,
    val sats: Long,
)

/**
 * Client for the cove-fake-node control API (`rust/crates/cove-fake-node/README.md`)
 *
 * Start the node with `cargo xtask fake-node run --android -- <command>` or
 * `just android-instrumented-ci`, which forward its ports into the device
 */
class FakeNodeControl(
    host: String = "127.0.0.1",
    port: Int = 39_000,
) {
    private val http =
        HostControlHttp(host, port) {
            "cove-fake-node control API at $host:$port is unreachable; run the test through " +
                "`just android-instrumented-ci` or `cargo xtask fake-node run --android -- <command>`"
        }

    /**
     * Replaces the chain with a fresh one and clears the broadcast log
     *
     * With [network] the new chain starts from that network's genesis block instead of the one the
     * node was started on, until the next reset without it
     */
    fun reset(network: Network? = null) {
        val body = JSONObject()
        network?.let { body.put("network", it.fakeNodeName()) }

        request("POST", "/reset", body)
    }

    /** Pays each amount to the address in its own confirmed transaction */
    fun fund(
        address: String,
        sats: List<Long>,
    ): List<FakeNodeFundedOutput> {
        val body =
            JSONObject()
                .put("address", address)
                .put("sats", JSONArray(sats))
                .put("confirm", true)

        return fundedOutputs(request("POST", "/fund", body))
    }

    /**
     * Funds the receive addresses `index` of the mnemonic's first account of [addressType], one
     * transaction per amount, leaving them in the mempool unless [confirm]
     */
    fun fundWallet(
        mnemonic: String,
        satsByIndex: List<Pair<Int, Long>>,
        addressType: WalletAddressType = WalletAddressType.NATIVE_SEGWIT,
        confirm: Boolean = true,
    ): List<FakeNodeFundedOutput> {
        val outputs = JSONArray()
        satsByIndex.forEach { (index, sats) ->
            outputs.put(JSONObject().put("index", index).put("sats", sats))
        }

        val body =
            JSONObject()
                .put("mnemonic", mnemonic)
                .put("address_type", addressType.fakeNodeName())
                .put("outputs", outputs)
                .put("confirm", confirm)

        return fundedOutputs(request("POST", "/fund-wallet", body))
    }

    /** Mines blocks, the first confirming every mempool transaction, and returns the new tip */
    fun mine(blocks: Int = 1): Int = JSONObject(request("POST", "/mine", JSONObject().put("blocks", blocks))).getInt("tip")

    fun broadcasts(): List<FakeNodeBroadcast> =
        JSONArray(request("GET", "/broadcasts")).objects().map { broadcast ->
            FakeNodeBroadcast(
                txid = broadcast.getString("txid"),
                inputs = broadcast.getJSONArray("inputs").strings().map(FakeNodeOutPoint::parse),
                outputs =
                    broadcast.getJSONArray("outputs").objects().map { output ->
                        FakeNodeTxOut(
                            address = output.optString("address").takeUnless { output.isNull("address") },
                            sats = output.getLong("sats"),
                        )
                    },
            )
        }

    fun isSpent(outpoint: FakeNodeOutPoint): Boolean = outspend(outpoint).getBoolean("spent")

    /** The transaction spending the output, pending or confirmed, or null while it is unspent */
    fun spendingTxid(outpoint: FakeNodeOutPoint): String? =
        outspend(outpoint).let { if (it.isNull("spending_txid")) null else it.getString("spending_txid") }

    private fun outspend(outpoint: FakeNodeOutPoint): JSONObject =
        JSONObject(request("GET", "/outspend/${outpoint.txid}/${outpoint.vout}"))

    private fun request(
        method: String,
        path: String,
        body: JSONObject? = null,
    ): String = http.request(method, path, body)

    private fun fundedOutputs(response: String): List<FakeNodeFundedOutput> =
        JSONObject(response).getJSONArray("outputs").objects().map { output ->
            FakeNodeFundedOutput(
                outpoint = FakeNodeOutPoint(output.getString("txid"), output.getInt("vout")),
                sats = output.getLong("sats"),
                address = output.getString("address"),
            )
        }

    private fun Network.fakeNodeName(): String =
        when (this) {
            Network.BITCOIN -> "bitcoin"
            Network.TESTNET -> "testnet"
            Network.TESTNET4 -> "testnet4"
            Network.SIGNET -> "signet"
        }

    private fun WalletAddressType.fakeNodeName(): String =
        when (this) {
            WalletAddressType.NATIVE_SEGWIT -> "native_segwit"
            WalletAddressType.WRAPPED_SEGWIT -> "wrapped_segwit"
            WalletAddressType.LEGACY -> "legacy"
        }

    private fun JSONArray.objects(): List<JSONObject> = (0 until length()).map(::getJSONObject)

    private fun JSONArray.strings(): List<String> = (0 until length()).map(::getString)

}
