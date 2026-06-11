package org.bitcoinppl.cove.flows.OnboardingFlow

import org.junit.Assert.assertEquals
import org.junit.Test

class OnboardingBackupViewsTest {
    @Test
    fun recoveryWordsUseFirstHalfInLeftColumn() {
        val words = (1..12).map { "word-$it" }

        val orderedWords = onboardingWordsInTwoColumnVisualOrder(words)

        assertEquals(
            listOf(
                OnboardingWordCardItem(index = 1, word = "word-1"),
                OnboardingWordCardItem(index = 7, word = "word-7"),
                OnboardingWordCardItem(index = 2, word = "word-2"),
                OnboardingWordCardItem(index = 8, word = "word-8"),
                OnboardingWordCardItem(index = 3, word = "word-3"),
                OnboardingWordCardItem(index = 9, word = "word-9"),
                OnboardingWordCardItem(index = 4, word = "word-4"),
                OnboardingWordCardItem(index = 10, word = "word-10"),
                OnboardingWordCardItem(index = 5, word = "word-5"),
                OnboardingWordCardItem(index = 11, word = "word-11"),
                OnboardingWordCardItem(index = 6, word = "word-6"),
                OnboardingWordCardItem(index = 12, word = "word-12"),
            ),
            orderedWords,
        )
    }
}
