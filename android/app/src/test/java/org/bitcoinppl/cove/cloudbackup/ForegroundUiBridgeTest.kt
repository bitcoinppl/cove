package org.bitcoinppl.cove.cloudbackup

import android.content.Intent
import android.content.IntentSender
import androidx.activity.result.ActivityResult
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContract
import androidx.activity.result.IntentSenderRequest
import androidx.core.app.ActivityOptionsCompat
import androidx.fragment.app.FragmentActivity
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import kotlinx.coroutines.withTimeout
import org.bitcoinppl.cove_core.device.PasskeyException
import org.bitcoinppl.cove_core.device.PasskeyOperation
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import java.util.concurrent.atomic.AtomicBoolean

@OptIn(ExperimentalCoroutinesApi::class)
class ForegroundUiBridgeTest {
    private val mainDispatcher = UnconfinedTestDispatcher()

    @Before
    fun setUp() {
        Dispatchers.setMain(mainDispatcher)
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    @Test
    fun pauseKeepsPendingAuthorizationResultDeliverable() =
        runBlocking {
            val activity = unsafeInstance<FragmentActivity>()
            val launcher = RecordingAuthorizationLauncher()
            ForegroundUiBridge.attach(activity, launcher)

            val received = async { ForegroundUiBridge.launchAuthorization(intentSenderRequest()) }
            launcher.launched.await()

            ForegroundUiBridge.pause(activity)
            val result = ActivityResult(RESULT_OK, Intent("drive.authorization"))
            ForegroundUiBridge.handleAuthorizationResult(result)

            assertEquals(result, received.await())
            ForegroundUiBridge.detach(activity)
        }

    @Test
    fun authorizationWaitRemainsPendingWithoutAWatchdog() =
        runBlocking {
            val activity = unsafeInstance<FragmentActivity>()
            val launcher = RecordingAuthorizationLauncher()
            ForegroundUiBridge.attach(activity, launcher)

            val received = async { ForegroundUiBridge.launchAuthorization(intentSenderRequest()) }
            launcher.launched.await()

            mainDispatcher.scheduler.advanceTimeBy(5 * 60_000L)

            assertFalse(received.isCompleted)

            val result = ActivityResult(RESULT_OK, Intent("drive.authorization"))
            ForegroundUiBridge.handleAuthorizationResult(result)

            assertEquals(result, received.await())
            ForegroundUiBridge.detach(activity)
        }

    @Test
    fun detachCancelsPendingAuthorizationResultAfterPause() =
        runBlocking {
            val activity = unsafeInstance<FragmentActivity>()
            val launcher = RecordingAuthorizationLauncher()
            ForegroundUiBridge.attach(activity, launcher)

            val received = async { ForegroundUiBridge.launchAuthorization(intentSenderRequest()) }
            launcher.launched.await()

            ForegroundUiBridge.pause(activity)
            ForegroundUiBridge.detach(activity)

            val cancellation =
                try {
                    received.await()
                    null
                } catch (error: CancellationException) {
                    error
                }

            assertNotNull(cancellation)
        }

    @Test
    fun presenterIsReadyOnlyWhileAnActivityIsResumed() =
        runBlocking {
            withTimeout(TEST_DEADLINE_MS) {
                val activity = unsafeInstance<FragmentActivity>()

                assertEquals(
                    PresenterReadiness.NotReady,
                    ForegroundUiBridge.awaitPresenter(timeoutMs = 50, isAppLocked = UNLOCKED),
                )

                ForegroundUiBridge.attach(activity, RecordingAuthorizationLauncher())
                assertEquals(
                    PresenterReadiness.Ready(activity),
                    ForegroundUiBridge.awaitPresenter(timeoutMs = 50, isAppLocked = UNLOCKED),
                )

                ForegroundUiBridge.pause(activity)
                assertEquals(
                    PresenterReadiness.NotReady,
                    ForegroundUiBridge.awaitPresenter(timeoutMs = 50, isAppLocked = UNLOCKED),
                )

                ForegroundUiBridge.detach(activity)
            }
        }

    @Test
    fun presenterBecomesReadyWhenAnActivityResumesDuringTheWait() =
        runBlocking {
            withTimeout(TEST_DEADLINE_MS) {
                val activity = unsafeInstance<FragmentActivity>()
                val readiness =
                    async(Dispatchers.Default) {
                        ForegroundUiBridge.awaitPresenter(timeoutMs = 5_000, isAppLocked = UNLOCKED)
                    }

                delay(50)
                ForegroundUiBridge.attach(activity, RecordingAuthorizationLauncher())

                assertEquals(PresenterReadiness.Ready(activity), readiness.await())
                ForegroundUiBridge.detach(activity)
            }
        }

    /**
     * The app lock is a Compose window that leaves the activity resumed, so only the lock state
     * keeps a resumed activity from counting as ready. A locked app is not ready at once and does
     * not hold the request until the user unlocks it
     */
    @Test
    fun lockedResumedActivityIsNotReadyAtOnceEvenWhenItUnlocksDuringTheWait() =
        runBlocking {
            withTimeout(TEST_DEADLINE_MS) {
                val activity = unsafeInstance<FragmentActivity>()
                val locked = AtomicBoolean(true)
                ForegroundUiBridge.attach(activity, RecordingAuthorizationLauncher())

                val unlock =
                    launch(Dispatchers.Default) {
                        delay(50)
                        locked.set(false)
                    }
                val started = System.nanoTime()
                val readiness = ForegroundUiBridge.awaitPresenter(timeoutMs = 5_000, isAppLocked = locked::get)
                val elapsedMs = (System.nanoTime() - started) / 1_000_000
                unlock.cancel()

                assertEquals(PresenterReadiness.NotReady, readiness)
                assertTrue("a locked app must not wait out the timeout, took ${elapsedMs}ms", elapsedMs < 1_000)
                ForegroundUiBridge.detach(activity)
            }
        }

    /**
     * A request that arrives while the app is locked and in the background is refused at once, so
     * an unlock and a resumed activity within the wait never start it without a new tap
     */
    @Test
    fun lockedRequestWithNoActivityIsNotStartedByALaterUnlockAndResume() =
        runBlocking {
            withTimeout(TEST_DEADLINE_MS) {
                val activity = unsafeInstance<FragmentActivity>()
                val locked = AtomicBoolean(true)
                var starts = 0
                val presenter =
                    PasskeyPresenter(
                        awaitPresenter = {
                            ForegroundUiBridge.awaitPresenter(timeoutMs = 5_000, isAppLocked = locked::get)
                        },
                        requestMayPresent = { true },
                    )

                val request =
                    async {
                        runCatching {
                            presenter.interactive(PasskeyOperation.AUTHENTICATE_ASSERTION) { starts += 1 }
                        }.exceptionOrNull()
                    }

                // the user unlocks and the activity resumes well within the wait
                delay(50)
                locked.set(false)
                ForegroundUiBridge.attach(activity, RecordingAuthorizationLauncher())

                val error = request.await()
                assertTrue("expected NotReady, got ${error?.javaClass?.name}", error is PasskeyException.NotReady)
                assertEquals(0, starts)

                // a request from a new tap, unlocked and resumed, starts
                presenter.interactive(PasskeyOperation.AUTHENTICATE_ASSERTION) { requestActivity ->
                    assertSame(activity, requestActivity)
                    starts += 1
                }

                assertEquals(1, starts)
                ForegroundUiBridge.detach(activity)
            }
        }

    @Test
    fun passkeyRequestStartsOnlyWhileTheResumedActivityIsUnlocked() =
        runBlocking {
            withTimeout(TEST_DEADLINE_MS) {
                val activity = unsafeInstance<FragmentActivity>()
                val locked = AtomicBoolean(true)
                var starts = 0
                val presenter =
                    PasskeyPresenter(
                        awaitPresenter = { ForegroundUiBridge.awaitPresenter(isAppLocked = locked::get) },
                        requestMayPresent = { true },
                    )
                ForegroundUiBridge.attach(activity, RecordingAuthorizationLauncher())

                val error =
                    runCatching {
                        presenter.interactive(PasskeyOperation.AUTHENTICATE_ASSERTION) { starts += 1 }
                    }.exceptionOrNull()

                // the generated toString calls into Rust, which JVM unit tests cannot load, so name only the class
                assertTrue("expected NotReady, got ${error?.javaClass?.name}", error is PasskeyException.NotReady)
                assertEquals(PasskeyOperation.AUTHENTICATE_ASSERTION, (error as PasskeyException.NotReady).operation)
                assertEquals(0, starts)

                locked.set(false)
                presenter.interactive(PasskeyOperation.AUTHENTICATE_ASSERTION) { requestActivity ->
                    assertSame(activity, requestActivity)
                    starts += 1
                }

                assertEquals(1, starts)
                ForegroundUiBridge.detach(activity)
            }
        }

    private class RecordingAuthorizationLauncher :
        ActivityResultLauncher<IntentSenderRequest>() {
        val launched = CompletableDeferred<IntentSenderRequest>()

        override val contract: ActivityResultContract<IntentSenderRequest, *> =
            object : ActivityResultContract<IntentSenderRequest, ActivityResult>() {
                override fun createIntent(
                    context: android.content.Context,
                    input: IntentSenderRequest,
                ): Intent = Intent()

                override fun parseResult(
                    resultCode: Int,
                    intent: Intent?,
                ): ActivityResult = ActivityResult(resultCode, intent)
            }

        override fun launch(
            input: IntentSenderRequest,
            options: ActivityOptionsCompat?,
        ) {
            launched.complete(input)
        }

        override fun unregister() = Unit
    }

    private companion object {
        const val RESULT_OK = -1

        // fails a wait that never ends instead of hanging the suite
        const val TEST_DEADLINE_MS = 10_000L

        // the production lock source is the app's AuthManager, which JVM unit tests cannot create
        val UNLOCKED: () -> Boolean = { false }

        fun intentSenderRequest(): IntentSenderRequest =
            IntentSenderRequest.Builder(intentSender()).build()

        fun intentSender(): IntentSender = unsafeInstance()
    }
}
