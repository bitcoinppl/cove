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
import org.bitcoinppl.cove.AutoComplete
open class Bip39AutoComplete : org.bitcoinppl.cove.Bip39AutoComplete(), AutoComplete {}

@Composable
fun CoveApp(viewModel: ViewModel = viewModel()) {
    Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.Center
    ) {
        Button(
            onClick = { }
        ) {
            Text(text = Bip39AutoComplete().autocomplete(word = "da")[0], color = Color.White, fontSize = 32.sp)
        }

        AutocompleteField(autocompleter = Bip39AutoComplete(), text = "ab", onTextChange = {})
    }
}

@Composable
fun <AutoCompleter : AutoComplete> AutocompleteField(
    autocompleter: AutoCompleter,
    text: String,
    onTextChange: (String) -> Unit,
) {
    Text(text = autocompleter.autocomplete(text)[0])
}

@Preview(showBackground = true)
@Composable
fun DefaultPreview() {
    CoveTheme {
        CoveApp()
    }
}
