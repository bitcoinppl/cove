package com.example.cove

import androidx.compose.foundation.layout.*
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.example.cove.ui.theme.CoveTheme
import androidx.lifecycle.viewmodel.compose.viewModel
import uniffi.cove.Event

@Composable
fun TimerApp(viewModel: com.example.cove.ViewModel = viewModel()) {
    val timer by viewModel.timer.collectAsState()

    Column(modifier = Modifier.padding(16.dp)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center
        ) {
            Text(
                text = "${timer.elapsedSecs}",
                fontSize = 32.sp,
                modifier = Modifier.padding(horizontal = 16.dp)
            )
        }
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center
        ) {
            if (timer.active) {
                Button(onClick = { viewModel.dispatch(Event.TIMER_PAUSE) }) {
                    Text(text = "Stop")
                }
            } else {
                Button(onClick = { viewModel.dispatch(Event.TIMER_START) }) {
                    Text(text = "Start")
                }
            }
            Button(onClick = { viewModel.dispatch(Event.TIMER_RESET) }) {
                Text(text = "Reset")
            }
        }
    }
}

@Preview(showBackground = true)
@Composable
fun DefaultPreview() {
    CoveTheme {
        TimerApp()
    }
}
