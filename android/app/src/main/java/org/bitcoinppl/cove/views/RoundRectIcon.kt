package org.bitcoinppl.cove.views

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Person
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp


@Composable
fun RoundRectIcon(
    size: Dp? = 40.dp,
    backgroundColor: Color? = Color.Gray,
    imageVector: ImageVector,
    contentDescription: String? = null,
    cornerRadius: Dp = 4.dp,
    imageTint: Color = Color.White
) {
    Box(
        modifier = Modifier
            .size(size!!)
            .background(color = backgroundColor!!, shape = RoundedCornerShape(cornerRadius)),
        contentAlignment = Alignment.Center
    ) {
        Icon(
            imageVector = imageVector,
            contentDescription = contentDescription,
            tint = imageTint,
            modifier = Modifier.fillMaxSize(0.7f)
        )

    }
}

@Preview
@Composable
fun SamplePreview() {
    RoundRectIcon(
        imageVector = Icons.Default.Person,
        contentDescription = "Profile Icon",
    )
}
