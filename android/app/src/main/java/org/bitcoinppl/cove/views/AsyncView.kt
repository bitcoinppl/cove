package org.bitcoinppl.cove.views

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.Log
import kotlin.coroutines.cancellation.CancellationException

// text that shows a loading spinner when the value is null
@Composable
fun AsyncText(
    text: String?,
    modifier: Modifier = Modifier,
    color: Color = Color.Unspecified,
    style: TextStyle = LocalTextStyle.current,
    fontWeight: FontWeight? = null,
    spinnerSize: Dp = 16.dp,
    spinnerStrokeWidth: Dp = 2.dp,
) {
    if (text != null) {
        Text(
            text = text,
            modifier = modifier,
            color = color,
            style = style,
            fontWeight = fontWeight,
        )
    } else {
        CircularProgressIndicator(
            modifier = modifier.size(spinnerSize),
            strokeWidth = spinnerStrokeWidth,
            color = if (color != Color.Unspecified) color else Color.Gray,
        )
    }
}

// view that runs an async operation and shows loading/content/error states
// if cachedValue is provided, show it immediately while async operation runs
@Composable
fun <T> AsyncView(
    cachedValue: T? = null,
    operation: suspend () -> T,
    modifier: Modifier = Modifier,
    errorView: @Composable () -> Unit = {},
    content: @Composable (T) -> Unit,
) {
    var result by remember { mutableStateOf<Result<T>?>(null) }

    LaunchedEffect(Unit) {
        result =
            try {
                Result.success(operation())
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.e("AsyncView", "Error loading async view: ${e.message}", e)
                Result.failure(e)
            }
    }

    Box(modifier = modifier, contentAlignment = Alignment.Center) {
        when (val r = result) {
            null -> {
                if (cachedValue != null) {
                    content(cachedValue)
                } else {
                    CircularProgressIndicator(
                        modifier = Modifier.size(16.dp),
                        strokeWidth = 2.dp,
                    )
                }
            }
            else -> {
                if (r.isSuccess) {
                    content(r.getOrThrow())
                } else if (cachedValue != null) {
                    content(cachedValue)
                } else {
                    errorView()
                }
            }
        }
    }
}
