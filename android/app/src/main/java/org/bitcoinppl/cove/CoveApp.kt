import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.sp
import com.example.cove.Manager
import org.bitcoinppl.cove_core.AutoComplete
import org.bitcoinppl.cove_core.Bip39AutoComplete
import org.bitcoinppl.cove.ui.theme.CoveTheme

@Composable
fun CoveApp(Manager: Manager = Manager()) {
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
