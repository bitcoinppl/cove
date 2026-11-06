package org.bitcoinppl.cove.testconfig

import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/**
 * A one-shot hold a test puts on the next call of one kind
 *
 * The call that claims it parks until the test releases it, or until its process is killed,
 * which is how staged process tests interrupt the app mid-call
 */
class TestHold {
    private val armed = AtomicReference<Gate?>(null)

    @Volatile
    private var claimed: Gate? = null

    /** Holds the next call that checks this hold */
    fun holdNext() {
        armed.set(Gate())
    }

    /** Parks the calling thread if a hold is armed, claiming it so later calls pass */
    fun enterIfHeld() {
        val gate = armed.getAndSet(null) ?: return
        claimed = gate
        gate.entered.countDown()
        gate.released.await()
    }

    /** Whether a call entered the armed hold within [timeoutMs] */
    fun awaitEntered(timeoutMs: Long = 20_000): Boolean {
        val gate = armed.get() ?: claimed ?: return false

        return gate.entered.await(timeoutMs, TimeUnit.MILLISECONDS)
    }

    /** Lets a held call continue and disarms a hold no call has claimed yet */
    fun release() {
        armed.getAndSet(null)?.released?.countDown()
        claimed?.released?.countDown()
        claimed = null
    }

    private class Gate {
        val entered = CountDownLatch(1)
        val released = CountDownLatch(1)
    }
}
