package org.bitcoinppl.cove

import androidx.lifecycle.ViewModel

/**
 * base manager for Compose screens
 * provides lifecycle-aware coroutine scope and cleanup
 * matches Swift naming convention where ViewModels are called "Manager"
 */
abstract class Manager : ViewModel() {
    // viewModelScope is already provided by ViewModel and is automatically cancelled when ViewModel is cleared

    /**
     * called when the manager is being destroyed
     * override this to cleanup resources
     */
    override fun onCleared() {
        super.onCleared()
    }
}
