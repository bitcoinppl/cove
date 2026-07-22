package org.bitcoinppl.cove.testconfig

import org.bitcoinppl.cove_core.device.DiscoveredPasskeyResult
import org.bitcoinppl.cove_core.device.PasskeyCredentialPresence
import org.bitcoinppl.cove_core.device.PasskeyException
import org.bitcoinppl.cove_core.device.PasskeyFailureReason
import org.bitcoinppl.cove_core.device.PasskeyOperation
import org.bitcoinppl.cove_core.device.PasskeyProvider
import org.bitcoinppl.cove_core.device.PasskeyRegistrationPlatform
import org.bitcoinppl.cove_core.device.PasskeyRegistrationResult
import org.bitcoinppl.cove_core.device.PasskeyRegistrationUser
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

object ScriptedPasskeyProvider : PasskeyProvider {
    private const val CALL_COUNT_POLL_INTERVAL_MS = 10L
    private val credentialId = "ui-test-passkey".encodeToByteArray()
    private val prfOutput = ByteArray(32) { index -> (index + 1).toByte() }

    @Volatile
    private var creation = CountDownLatch(1)

    @Volatile
    private var createResults = ResultQueue.success()

    @Volatile
    private var authenticationResults = ResultQueue.success()

    @Volatile
    private var discoveryResults = ResultQueue.success()

    private val createCalls = AtomicInteger(0)
    private val authenticationCalls = AtomicInteger(0)
    private val discoveryCalls = AtomicInteger(0)

    enum class Result {
        SUCCESS,
        PRE_PRESENTATION_FAILURE,
        POST_PRESENTATION_FAILURE,
        USER_CANCELLED,
        NO_CREDENTIAL,
    }

    enum class Invocation {
        CREATE,
        AUTHENTICATE,
        DISCOVER,
    }

    fun reset() {
        creation = CountDownLatch(1)
        createResults = ResultQueue.success()
        authenticationResults = ResultQueue.success()
        discoveryResults = ResultQueue.success()
        createCalls.set(0)
        authenticationCalls.set(0)
        discoveryCalls.set(0)
    }

    fun configureResults(
        invocation: Invocation,
        vararg results: Result,
    ) {
        val queue = ResultQueue.from(results)

        when (invocation) {
            Invocation.CREATE -> createResults = queue
            Invocation.AUTHENTICATE -> authenticationResults = queue
            Invocation.DISCOVER -> discoveryResults = queue
        }
    }

    fun awaitCreation(timeoutMs: Long = 10_000): Boolean =
        creation.await(timeoutMs, TimeUnit.MILLISECONDS)

    fun callCount(invocation: Invocation): Int =
        when (invocation) {
            Invocation.CREATE -> createCalls.get()
            Invocation.AUTHENTICATE -> authenticationCalls.get()
            Invocation.DISCOVER -> discoveryCalls.get()
        }

    fun awaitCallCount(
        invocation: Invocation,
        expected: Int,
        timeoutMs: Long = 10_000,
    ): Boolean {
        val deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMs)

        while (System.nanoTime() < deadline) {
            if (callCount(invocation) >= expected) return true

            Thread.sleep(CALL_COUNT_POLL_INTERVAL_MS)
        }

        return callCount(invocation) >= expected
    }

    override fun createPasskey(
        rpId: String,
        challenge: ByteArray,
        user: PasskeyRegistrationUser,
    ): PasskeyRegistrationResult {
        createCalls.incrementAndGet()
        creation.countDown()
        createResults.next().throwIfFailed(PasskeyOperation.REGISTRATION)

        return PasskeyRegistrationResult(
            credentialId = credentialId.copyOf(),
            providerAaguid = "",
            registeredPlatform = PasskeyRegistrationPlatform.ANDROID,
        )
    }

    override fun authenticateWithPrf(
        rpId: String,
        credentialId: ByteArray,
        prfSalt: ByteArray,
        challenge: ByteArray,
    ): ByteArray {
        authenticationCalls.incrementAndGet()
        authenticationResults.next().throwIfFailed(PasskeyOperation.AUTHENTICATE_ASSERTION)

        return prfOutput.copyOf()
    }

    override fun discoverAndAuthenticateWithPrf(
        rpId: String,
        prfSalt: ByteArray,
        challenge: ByteArray,
    ): DiscoveredPasskeyResult {
        discoveryCalls.incrementAndGet()
        discoveryResults.next().throwIfFailed(PasskeyOperation.DISCOVER_ASSERTION)

        return DiscoveredPasskeyResult(
            prfOutput = prfOutput.copyOf(),
            credentialId = credentialId.copyOf(),
        )
    }

    override fun isPrfSupported(): Boolean = true

    override fun checkPasskeyPresence(
        rpId: String,
        credentialId: ByteArray,
    ): PasskeyCredentialPresence = PasskeyCredentialPresence.PRESENT

    private fun Result.throwIfFailed(operation: PasskeyOperation) {
        val failure =
            when (this) {
                Result.SUCCESS -> null
                Result.PRE_PRESENTATION_FAILURE ->
                    PasskeyException.RequestFailed(
                        operation,
                        PasskeyFailureReason.PlatformAuthorizationFailed,
                    )
                Result.POST_PRESENTATION_FAILURE ->
                    PasskeyException.RequestFailed(
                        operation,
                        PasskeyFailureReason.PlatformAuthorizationFailedAfterPresentation,
                    )
                Result.USER_CANCELLED -> PasskeyException.UserCancelled()
                Result.NO_CREDENTIAL -> PasskeyException.NoCredentialFound()
            }

        if (failure != null) throw failure
    }

    private class ResultQueue(results: List<Result>) {
        private val queued = ConcurrentLinkedQueue(results)
        private val fallback = results.last()

        fun next(): Result = queued.poll() ?: fallback

        companion object {
            fun success() = ResultQueue(listOf(Result.SUCCESS))

            fun from(results: Array<out Result>): ResultQueue {
                require(results.isNotEmpty()) { "at least one passkey result is required" }

                return ResultQueue(results.toList())
            }
        }
    }
}
