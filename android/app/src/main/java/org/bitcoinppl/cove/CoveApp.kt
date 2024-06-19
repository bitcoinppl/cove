import androidx.compose.foundation.layout.*
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.example.cove.ViewModel
import com.example.cove.ui.theme.CoveTheme
import androidx.lifecycle.viewmodel.compose.viewModel
import uniffi.cove.Event


@Composable
fun CoveApp(viewModel: ViewModel = viewModel()) {
    val count by viewModel.cove.collectAsState()
    Box(
        modifier = Modifier
            .fillMaxSize(),
        contentAlignment = Alignment.Center
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center
        ) {
            Button(
                onClick = { viewModel.dispatch(Event.DECREMENT) },
                colors = ButtonDefaults.buttonColors(containerColor = Color.Red),
                modifier = Modifier
                    .size(64.dp)
            ) {
                Text("-", color = Color.White, fontSize = 40.sp)
            }

            Text(
                text = "$count",
                fontSize = 32.sp,
                modifier = Modifier.padding(horizontal = 16.dp)
            )

            Button(
                onClick = { viewModel.dispatch(Event.INCREMENT) },
                colors = ButtonDefaults.buttonColors(containerColor = Color.Green),
                modifier = Modifier
                    .size(64.dp)
            ) {
                Text("+", color = Color.White, fontSize = 32.sp)
            }
        }
    }
}

@Preview(showBackground = true)
@Composable
fun DefaultPreview() {
    CoveTheme {
        CoveApp()
    }
}
