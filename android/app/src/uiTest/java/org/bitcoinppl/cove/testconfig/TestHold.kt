package org.bitcoinppl.cove.testconfig

import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

/**
 * A one-shot hold a test puts on the next call of one kind
 *
 * The call that claims it parks until the test releases it, or until its process is killed,
 * which is how staged process tests interrupt the app mid-call
 */
class TestHold {
    // the gate stays set from holdNext until release, so a waiting test always finds it, and only
    // the first call to reach it claims it
    private val current = AtomicReference<Gate?>(null)

    /** Holds the next call that checks this hold */
    fun holdNext() {
        current.getAndSet(Gate())?.released?.countDown()
    }

    /** Parks the calling thread if a hold is armed, claiming it so later calls pass */
    fun enterIfHeld() {
        val gate = current.get() ?: return
        if (!gate.claimed.compareAndSet(false, true)) return

        gate.entered.countDown()
        gate.released.await()
    }

    /** Whether a call entered the armed hold within [timeoutMs] */
    fun awaitEntered(timeoutMs: Long = 20_000): Boolean {
        val gate = current.get() ?: return false

        return gate.entered.await(timeoutMs, TimeUnit.MILLISECONDS)
    }

    /** Lets a held call continue and disarms a hold no call has claimed yet */
    fun release() {
        current.getAndSet(null)?.released?.countDown()
    }

    private class Gate {
        val claimed = AtomicBoolean(false)
        val entered = CountDownLatch(1)
        val released = CountDownLatch(1)
    }
}
