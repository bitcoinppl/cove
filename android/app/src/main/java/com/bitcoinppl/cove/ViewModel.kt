package com.example.cove

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import uniffi.cove.Event
import uniffi.cove.FfiApp
import uniffi.cove.FfiUpdater
import uniffi.cove.TimerState
import uniffi.cove.Update

class ViewModel : ViewModel(), FfiUpdater  {
    private val rust: FfiApp = FfiApp()

    private var _cove: MutableStateFlow<Int>
    val cove: StateFlow<Int> get() = _cove

    private var _timer: MutableStateFlow<TimerState>
    val timer: StateFlow<TimerState> get() = _timer

    init {
        rust.listenForUpdates(this)

        val state = rust.getState()
        _cove = MutableStateFlow(state.count)
        _timer = MutableStateFlow(state.timer)
    }

    override fun update(update: Update) {
        when (update) {
            is Update.CountChanged -> {
                _cove.value = update.count
            }
            is Update.Timer -> {
                println("timer" + update.state)
                _timer.value = update.state
            }
        }
    }

    fun dispatch(event: Event) {
        rust.dispatch(event)
    }
}
