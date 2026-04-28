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
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Before
import org.junit.Test

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

        fun intentSenderRequest(): IntentSenderRequest =
            IntentSenderRequest.Builder(intentSender()).build()

        fun intentSender(): IntentSender = unsafeInstance()

        inline fun <reified T> unsafeInstance(): T {
            val field = sun.misc.Unsafe::class.java.getDeclaredField("theUnsafe")
            field.isAccessible = true
            val unsafe = field.get(null) as sun.misc.Unsafe
            return unsafe.allocateInstance(T::class.java) as T
        }
    }
}
