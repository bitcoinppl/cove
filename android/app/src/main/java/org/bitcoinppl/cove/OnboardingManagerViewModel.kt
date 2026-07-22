package org.bitcoinppl.cove

import androidx.lifecycle.ViewModel

internal class OnboardingManagerViewModel : ViewModel() {
    private var retainedManager: OnboardingManager? = null

    fun manager(app: AppManager): OnboardingManager =
        retainedManager ?: OnboardingManager(app).also { retainedManager = it }

    fun release() {
        val manager = retainedManager ?: return
        retainedManager = null
        manager.close()
    }

    override fun onCleared() {
        release()
    }
}
