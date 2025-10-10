package org.bitcoinppl.cove.views

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.ui.theme.*


@Preview
@Composable
fun ThemedSwitchPreview() {
    var isChecked by remember { mutableStateOf(false) }
    ThemedSwitch(isChecked) { isChecked = it }
}

@Composable
fun ThemedSwitch(isChecked: Boolean, onCheckChanged: ((Boolean) -> Unit)) {
    Switch(
        checked = isChecked,
        onCheckedChange = onCheckChanged,
        colors = SwitchDefaults.colors(
            checkedThumbColor = Color.White,
            checkedTrackColor = LinkBlue,
            uncheckedThumbColor = Color.White,
            uncheckedTrackColor = Color.LightGray,
        )
    )
}


@Composable
fun CustomSpacer(height: Dp? = 1.dp, paddingValues: PaddingValues) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .height(height!!)
    ) {
        Spacer(
            modifier = Modifier
                .height(1.dp)
                .fillMaxWidth()
                .padding(paddingValues)
                .background(Color.LightGray)
                .align(Alignment.CenterEnd)
        )
    }
}

@Preview
@Composable
fun CustomSpacerPreview() {
    CustomSpacer(paddingValues = PaddingValues(start = 54.dp))
}

@Composable
fun InfoRow(label: String, text: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = 6.dp, bottom = 6.dp, start = 8.dp, end = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceEvenly
    ) {
        Text(
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f),
            text = label,
            style = MaterialTheme.typography.bodyLarge,
            textAlign = TextAlign.Start,
        )
        Text(
            text = text,
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f),
            style = MaterialTheme.typography.bodyLarge,
            color = colorTextGray,
            textAlign = TextAlign.End,
        )
    }
}

@Preview
@Composable
fun InfoRowPreview() {
    InfoRow("Title Text", "Lorem ipsum")
}

@Composable
fun ClickableInfoRow(label: String, text: String, icon: ImageVector, onClick: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = 6.dp, bottom = 6.dp, start = 8.dp, end = 16.dp)
            .clickable(true, onClick = onClick),

        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f),
            text = label,
            style = MaterialTheme.typography.bodyLarge,
            textAlign = TextAlign.Start,
        )
        Text(
            text = text,
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f),
            style = MaterialTheme.typography.bodyLarge,
            color = colorTextGray,
            textAlign = TextAlign.End,
        )
        Icon(
            imageVector = icon,
            tint = colorGray,
            contentDescription = "Forward",
        )
    }
}

@Composable
fun CardItem(
    title: String,
    titleColor: Color? = colorTextGray,
    allCaps: Boolean? = false,
    content: @Composable() () -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
    ) {
        Spacer(modifier = Modifier.height(12.dp))
        Text(
            text = if (allCaps == true) title.uppercase() else title,
            style = MaterialTheme.typography.bodyLarge,
            color = titleColor!!,
            fontSize = 20.sp,
            modifier = Modifier
                .padding(horizontal = 8.dp, vertical = 4.dp)
        )
        Card(
            modifier = Modifier.fillMaxWidth(),
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainer,
            ),
            shape = RoundedCornerShape(size = 8.dp),
        ) {
            content()
        }
    }
}

@Preview
@Composable
fun CardItemPreview() {
    CardItem("name") { Text("hello") }
}

@Preview
@Composable
fun SwitchRowPreview() {
    SwitchRow("Switch", false, {})
}

@Composable
fun SwitchRow(
    label: String,
    switchCheckedState: Boolean = false,
    onCheckChanged: ((Boolean) -> Unit)? = null
) {
    Row(
        modifier = Modifier
            .fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically
    ) {

        Text(
            text = label,
            style = MaterialTheme.typography.bodyLarge,
            modifier = Modifier
                .weight(1f)
                .padding(horizontal = 8.dp)
        )

        ThemedSwitch(
            isChecked = switchCheckedState,
            onCheckChanged = onCheckChanged ?: {},
        )
    }
}
