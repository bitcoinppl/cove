package org.bitcoinppl.cove

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class OwnerScopedCommandTest {
    @Test
    fun callerCancellationDoesNotCancelOwnedCommandOrItsCompletionTransition() =
        runTest {
            val dispatcher = StandardTestDispatcher(testScheduler)
            val ownerScope = CoroutineScope(SupervisorJob() + dispatcher)
            val command = OwnerScopedCommand<Int>(ownerScope)
            val release = CompletableDeferred<Unit>()
            var completionTransitions = 0

            val caller =
                launch(dispatcher) {
                    command.start {
                        release.await()
                        completionTransitions += 1
                        42
                    }.await()
                }
            testScheduler.runCurrent()

            caller.cancel()
            testScheduler.runCurrent()
            release.complete(Unit)
            testScheduler.advanceUntilIdle()

            assertTrue(caller.isCancelled)
            assertEquals(1, completionTransitions)
            ownerScope.cancel()
        }

    @Test
    fun concurrentCallersShareOneOwnedCommand() =
        runTest {
            val dispatcher = StandardTestDispatcher(testScheduler)
            val ownerScope = CoroutineScope(SupervisorJob() + dispatcher)
            val command = OwnerScopedCommand<Int>(ownerScope)
            val release = CompletableDeferred<Unit>()
            var starts = 0

            val first = command.start {
                starts += 1
                release.await()
                7
            }
            val second = command.start { error("duplicate command must not start") }
            testScheduler.runCurrent()

            release.complete(Unit)
            testScheduler.advanceUntilIdle()

            assertTrue(first === second)
            assertEquals(1, starts)
            assertEquals(7, first.await())
            ownerScope.cancel()
        }
}
