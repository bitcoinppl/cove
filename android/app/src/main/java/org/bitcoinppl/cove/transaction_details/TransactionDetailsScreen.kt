package org.bitcoinppl.cove.transaction_details

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.NorthEast
import androidx.compose.material.icons.filled.SouthWest
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.example.cove.R
import org.bitcoinppl.cove.ui.theme.*
import org.bitcoinppl.cove.views.ImageButton
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.tooling.preview.Preview
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.min

enum class TxType { Sent, Received }

@Preview(showBackground = true)
@Composable
private fun TxDetailsSentLightPreview() {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 20.dp)
    ) {
        TransactionDetailsWidget(
            txType = TxType.Sent,
            isDark = false,
            address = "tb1qc39qku u3epx ww8th xmh7 qsw04 p3jgp kj3a he",
            addressExtra = "2,194,934 | 251,357",
            networkFeePrimary = "2,724 SATS",
            networkFeeSecondary = "≈ $3.15",
            recipientReceivesPrimary = "150,000 SATS",
            recipientReceivesSecondary = "≈ $173.86",
            totalSpentPrimary = "-152,724 SATS",
            totalSpentSecondary = "≈ $177.02",
        )
    }
}

@Preview(showBackground = true)
@Composable
private fun TxDetailsReceivedDarkPreview() {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .background(Color(0xFF000000))
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 20.dp)
        ) {
            TransactionDetailsWidget(
                txType = TxType.Received,
                isDark = true,
                address = "tb1qc39qku u3epx ww8th xmh7 qsw04 p3jgp kj3a he",
                addressExtra = "2,194,934 | 251,357",
                networkFeePrimary = "2,724 SATS",
                networkFeeSecondary = "≈ $3.15",
                recipientReceivesPrimary = "150,000 SATS",
                recipientReceivesSecondary = "≈ $173.86",
                totalSpentPrimary = "-152,724 SATS",
                totalSpentSecondary = "≈ $177.02",
            )
        }
    }
}

@Preview(showBackground = true)
@Composable
private fun TransactionSentLightPreview() {
    TransactionDetailsScreen(
        onBack = {},
        onAddLabel = {},
        onViewInExplorer = {},
        onShowDetails = {},
        isDark = false,
        txType = TxType.Sent,
        txAmountPrimary = "152,724 SATS",
        txAmountSecondary = "≈ $177.02",
        date = Date()
    )
}

@Preview(showBackground = true)
@Composable
private fun TransactionReceivedDarkPreview() {
    TransactionDetailsScreen(
        onBack = {},
        onAddLabel = {},
        onViewInExplorer = {},
        onShowDetails = {},
        isDark = true,
        txType = TxType.Received,
        txAmountPrimary = "152,724 SATS",
        txAmountSecondary = "≈ $177.02",
        date = Date()
    )
}

@Preview(showBackground = true)
@Composable
private fun TransactionSentLightExpandedPreview() {
    TransactionDetailsScreen(
        onBack = {},
        onAddLabel = {},
        onViewInExplorer = {},
        onShowDetails = {},
        isDark = false,
        txType = TxType.Sent,
        txAmountPrimary = "152,724 SATS",
        txAmountSecondary = "≈ $177.02",
        date = Date(),
        isExpanded = true,
        address = "tb1qc39qku u3epx ww8th xmh7 qsw04 p3jgp kj3a he",
        addressExtra = "2,194,934 | 251,357",
        networkFeePrimary = "2,724 SATS",
        networkFeeSecondary = "≈ $3.15",
        recipientReceivesPrimary = "150,000 SATS",
        recipientReceivesSecondary = "≈ $173.86",
        totalSpentPrimary = "-152,724 SATS",
        totalSpentSecondary = "≈ $177.02",
    )
}

@Preview(showBackground = true)
@Composable
private fun TransactionReceivedDarkExpandedPreview() {
    TransactionDetailsScreen(
        onBack = {},
        onAddLabel = {},
        onViewInExplorer = {},
        onShowDetails = {},
        isDark = true,
        txType = TxType.Received,
        txAmountPrimary = "152,724 SATS",
        txAmountSecondary = "≈ $177.02",
        date = Date(),
        isExpanded = true,
        address = "tb1qc39qku u3epx ww8th xmh7 qsw04 p3jgp kj3a he",
        addressExtra = "2,194,934 | 251,357",
        networkFeePrimary = "2,724 SATS",
        networkFeeSecondary = "≈ $3.15",
        recipientReceivesPrimary = "150,000 SATS",
        recipientReceivesSecondary = "≈ $173.86",
        totalSpentPrimary = "-152,724 SATS",
        totalSpentSecondary = "≈ $177.02",
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TransactionDetailsScreen(
    onBack: () -> Unit,
    onAddLabel: () -> Unit,
    onViewInExplorer: () -> Unit,
    onShowDetails: () -> Unit,
    onNetworkFeeInfo: () -> Unit = {},
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
    isDark: Boolean = false,
    txType: TxType = TxType.Sent,
    title: String? = null,
    txAmountPrimary: String = "",
    txAmountSecondary: String = "",
    date: Date? = null,
    isExpanded: Boolean = false,
    address: String = "",
    addressExtra: String? = null,
    networkFeePrimary: String? = null,
    networkFeeSecondary: String? = null,
    recipientReceivesPrimary: String? = null,
    recipientReceivesSecondary: String? = null,
    totalSpentPrimary: String? = null,
    totalSpentSecondary: String? = null,
) {
    val bg = if (isDark) Color(0xFF000000) else Color(0xFFFFFFFF)
    val fg = if (isDark) Color(0xFFEFEFEF) else Color(0xFF101010)
    val sub = if (isDark) Color(0xFFB8B8B8) else Color(0xFF8F8F95)
    val checkCircle = if (isDark) Color(0xFF0F0F12) else Color(0xFF0F1012)
    val chipBg = TransactionReceived

    val ringColors: List<Color> = if (isDark) {
        listOf(
            Color.White.copy(alpha = 0.60f),
            Color.White.copy(alpha = 0.35f),
            Color.White.copy(alpha = 0.18f),
        )
    } else {
        listOf(
            Color.Black.copy(alpha = 0.50f),
            Color.Black.copy(alpha = 0.30f),
            Color.Black.copy(alpha = 0.15f),
        )
    }

    val headerTitle = title ?: stringResource(
        id = if (txType == TxType.Sent) R.string.title_transaction_sent else R.string.title_transaction_received
    )
    val actionLabelRes =
        if (txType == TxType.Sent) R.string.label_transaction_sent else R.string.label_transaction_received
    val actionIcon = if (txType == TxType.Sent) Icons.Filled.NorthEast else Icons.Filled.SouthWest

    val dateFormatter = SimpleDateFormat("MMMM d, yyyy 'at' h:mm a", Locale.getDefault())
    val formattedDate = date?.let { dateFormatter.format(it) } ?: ""

    val message = if (formattedDate.isNotEmpty()) {
        stringResource(
            id = if (txType == TxType.Sent) R.string.label_transaction_sent_on else R.string.label_transaction_received_on,
            formattedDate
        )
    } else ""

    Scaffold(
        containerColor = bg,
        topBar = {
            CenterAlignedTopAppBar(
                colors = TopAppBarDefaults.centerAlignedTopAppBarColors(
                    containerColor = Color.Transparent,
                    titleContentColor = fg,
                    actionIconContentColor = fg,
                    navigationIconContentColor = fg,
                ),
                title = { Text("") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = null
                        )
                    }
                }
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) }
    ) { padding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
        ) {
            Image(
                painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier
                    .fillMaxHeight()
                    .align(Alignment.TopCenter)
                    .offset(y = (-60).dp)
                    .graphicsLayer(alpha = if (isDark) 0.75f else 0.15f),
            )

            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(horizontal = 20.dp)
                    .verticalScroll(rememberScrollState()),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Spacer(Modifier.height(16.dp))

                CheckWithRingsWidget(
                    diameter = 180.dp,
                    circleColor = checkCircle,
                    ringColors = ringColors,
                    checkColor = Color.White,
                )

                Spacer(Modifier.height(16.dp))

                Text(
                    headerTitle,
                    color = fg,
                    fontSize = 32.sp,
                    fontWeight = FontWeight.SemiBold,
                    lineHeight = 36.sp
                )

                Spacer(Modifier.height(4.dp))

                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier
                        .clip(RoundedCornerShape(16.dp))
                        .clickable { onAddLabel() }
                ) {
                    Box(
                        modifier = Modifier
                            .size(18.dp)
                            .clip(CircleShape)
                            .background(chipBg),
                        contentAlignment = Alignment.Center
                    ) {
                        Icon(
                            imageVector = Icons.Default.Add,
                            contentDescription = null,
                            tint = Color.White,
                            modifier = Modifier.size(14.dp)
                        )
                    }
                    Spacer(Modifier.size(8.dp))
                    Text(stringResource(R.string.btn_add_label), color = fg, fontSize = 16.sp)
                }

                Spacer(Modifier.height(24.dp))

                Text(
                    message,
                    color = sub,
                    fontSize = 16.sp,
                    textAlign = TextAlign.Center,
                    lineHeight = 22.sp
                )

                Spacer(Modifier.height(32.dp))

                Text(
                    txAmountPrimary,
                    color = fg,
                    fontSize = 36.sp,
                    fontWeight = FontWeight.ExtraBold,
                    lineHeight = 44.sp
                )

                Spacer(Modifier.height(4.dp))

                Text(
                    txAmountSecondary,
                    color = fg,
                    fontSize = 18.sp
                )

                Spacer(Modifier.height(32.dp))

                if (isDark) {
                    OutlinedButton(
                        onClick = {},
                        shape = RoundedCornerShape(24.dp),
                        border = BorderStroke(1.dp, fg),
                        colors = ButtonDefaults.outlinedButtonColors(contentColor = fg)
                    ) {
                        Row(
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Icon(actionIcon, contentDescription = null)
                            Spacer(Modifier.size(8.dp))
                            Text(
                                text = stringResource(actionLabelRes),
                                fontWeight = FontWeight.Normal,
                                fontSize = 16.sp
                            )
                        }
                    }
                } else {
                    Button(
                        onClick = {},
                        shape = RoundedCornerShape(24.dp),
                        colors = ButtonDefaults.buttonColors(
                            containerColor = Color.Black,
                            contentColor = Color.White
                        ),
                    ) {
                        Row(
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Icon(actionIcon, contentDescription = null)
                            Spacer(Modifier.size(8.dp))
                            Text(
                                text = stringResource(actionLabelRes),
                                fontWeight = FontWeight.Normal,
                                fontSize = 16.sp
                            )
                        }
                    }
                }

                if (isExpanded) {
                    TransactionDetailsWidget(
                        txType = txType,
                        isDark = isDark,
                        address = address,
                        addressExtra = addressExtra,
                        networkFeePrimary = networkFeePrimary,
                        networkFeeSecondary = networkFeeSecondary,
                        recipientReceivesPrimary = recipientReceivesPrimary,
                        recipientReceivesSecondary = recipientReceivesSecondary,
                        totalSpentPrimary = totalSpentPrimary,
                        totalSpentSecondary = totalSpentSecondary,
                        onNetworkFeeInfo = onNetworkFeeInfo,
                    )
                } else {
                    Spacer(Modifier.weight(1f))
                }

                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(bottom = 16.dp),
                    horizontalAlignment = Alignment.CenterHorizontally
                ) {
                    ImageButton(
                        text = stringResource(R.string.btn_view_in_explorer),
                        onClick = onViewInExplorer,
                        colors = ButtonDefaults.buttonColors(
                            containerColor = if (isDark) SurfaceDark else MidnightBlue,
                            contentColor = if (isDark) BorderLight else Color.White
                        ),
                        modifier = Modifier
                            .fillMaxWidth()
                    )

                    Spacer(Modifier.height(12.dp))

                    TextButton(
                        onClick = onShowDetails,
                        modifier = Modifier.align(Alignment.CenterHorizontally)
                    ) {
                        Text(
                            text = stringResource(if (isExpanded) R.string.btn_hide_details else R.string.btn_show_details),
                            color = sub,
                            fontSize = 14.sp,
                            textAlign = TextAlign.Center,
                            fontWeight = FontWeight.SemiBold
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun TransactionDetailsWidget(
    txType: TxType,
    isDark: Boolean,
    address: String,
    addressExtra: String?,
    networkFeePrimary: String?,
    networkFeeSecondary: String?,
    recipientReceivesPrimary: String?,
    recipientReceivesSecondary: String?,
    totalSpentPrimary: String?,
    totalSpentSecondary: String?,
    onNetworkFeeInfo: () -> Unit = {},
) {
    val dividerColor = if (isDark) Color(0xFF222428) else Color(0xFFE4E5E7)
    val sub = if (isDark) Color(0xFFB8B8B8) else Color(0xFF8F8F95)
    val fg = if (isDark) Color(0xFFEFEFEF) else Color(0xFF101010)

    Spacer(Modifier.height(48.dp))
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .height(1.dp)
            .background(dividerColor)
    )
    Spacer(Modifier.height(24.dp))

    val addressLabel =
        stringResource(if (txType == TxType.Sent) R.string.label_sent_to else R.string.label_received_from)
    if (address.isNotEmpty()) {
        Column(modifier = Modifier.fillMaxWidth()) {
            Text(
                addressLabel,
                color = if (isDark) Color(0xFFB8B8B8) else Color(0xFF6F6F75),
                fontSize = 16.sp
            )
            Spacer(Modifier.height(8.dp))
            Text(
                address,
                color = fg,
                fontSize = 20.sp,
                fontWeight = FontWeight.SemiBold,
                lineHeight = 24.sp
            )
            if (!addressExtra.isNullOrEmpty()) {
                Spacer(Modifier.height(8.dp))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(addressExtra, color = sub, fontSize = 14.sp)
                    Spacer(Modifier.size(4.dp))
                    Box(
                        modifier = Modifier
                            .size(14.dp)
                            .clip(CircleShape)
                            .background(Color(0xFF1FC35C)),
                        contentAlignment = Alignment.Center
                    ) {
                        Icon(
                            imageVector = Icons.Default.Check,
                            contentDescription = null,
                            tint = Color.White,
                            modifier = Modifier.size(10.dp)
                        )
                    }
                }
            }
        }
        Spacer(Modifier.height(24.dp))
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(dividerColor)
        )
        Spacer(Modifier.height(24.dp))
    }

    DetailsWidget(
        label = stringResource(R.string.label_network_fee),
        primary = networkFeePrimary,
        secondary = networkFeeSecondary,
        isDark = isDark,
        showInfoIcon = true,
        onInfoClick = onNetworkFeeInfo
    )
    Spacer(Modifier.height(24.dp))

    DetailsWidget(
        label = stringResource(R.string.label_recipient_receives),
        primary = recipientReceivesPrimary,
        secondary = recipientReceivesSecondary,
        isDark = isDark
    )
    Spacer(Modifier.height(24.dp))

    Box(
        modifier = Modifier
            .fillMaxWidth()
            .height(1.dp)
            .background(dividerColor)
    )
    Spacer(Modifier.height(24.dp))

    DetailsWidget(
        label = stringResource(R.string.label_total_spent),
        primary = totalSpentPrimary,
        secondary = totalSpentSecondary,
        isDark = isDark,
        isTotal = true
    )
    Spacer(Modifier.height(72.dp))
}

@Composable
private fun DetailsWidget(
    label: String,
    primary: String?,
    secondary: String?,
    isDark: Boolean,
    isTotal: Boolean = false,
    showInfoIcon: Boolean = false,
    onInfoClick: () -> Unit = {},
) {
    if (primary == null) return
    val sub = if (isDark) Color(0xFF8F8F95) else Color(0xFF6F6F75)
    val fg = if (isDark) Color(0xFFEFEFEF) else Color(0xFF101010)

    val labelColor = if (isTotal) {
        if (isDark) Color(0xFFEFEFEF) else Color(0xFF101010)
    } else {
        if (isDark) Color(0xFFB8B8B8) else Color(0xFF9CA3AF)
    }

    val primaryColor = if (isTotal) {
        fg
    } else {
        if (isDark) Color(0xFFB8B8B8) else Color(0xFF9CA3AF)
    }

    Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.weight(1f)
        ) {
            Text(
                label,
                color = labelColor,
                fontSize = 18.sp
            )
            if (showInfoIcon) {
                Spacer(Modifier.width(8.dp))
                IconButton(
                    onClick = onInfoClick,
                    modifier = Modifier.size(24.dp),
                    content = {
                        Icon(
                            imageVector = Icons.Outlined.Info,
                            contentDescription = null,
                            tint = if (isDark) Color(0xFFB8B8B8) else Color(0xFF9CA3AF),
                            modifier = Modifier.size(16.dp)
                        )
                    }
                )
            }
        }
        Column(horizontalAlignment = Alignment.End) {
            Text(primary, color = primaryColor, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
            if (!secondary.isNullOrEmpty()) {
                Spacer(Modifier.height(6.dp))
                Text(secondary, color = sub, fontSize = 14.sp)
            }
        }
    }
}


@Composable
private fun CheckWithRingsWidget(
    diameter: Dp,
    circleColor: Color,
    ringColors: List<Color>,
    checkColor: Color,
) {
    Box(
        contentAlignment = Alignment.Center,
        modifier = Modifier.size(diameter)
    ) {
        Canvas(modifier = Modifier.matchParentSize()) {
            val canvasMin = min(size.width, size.height)
            val stroke = 1.dp.toPx()

            val centerRadius = canvasMin * 0.35f
            val maxRadius = (canvasMin / 2f) - (stroke / 2f)
            val ringCount = ringColors.size
            val totalExtra = (maxRadius - centerRadius).coerceAtLeast(0f)
            val spacing = if (ringCount > 0) totalExtra / ringCount else 0f

            ringColors.forEachIndexed { index, color ->
                val r = centerRadius + ((index + 1) * spacing)
                drawCircle(
                    color = color,
                    radius = r,
                    style = Stroke(
                        width = stroke,
                        cap = StrokeCap.Round
                    )
                )
            }
        }
        Box(
            modifier = Modifier
                .size(diameter * 0.7f)
                .clip(CircleShape)
                .background(circleColor),
            contentAlignment = Alignment.Center
        ) {
            Canvas(modifier = Modifier.size(diameter * 0.36f)) {
                val stroke = 3.dp.toPx()
                val w = size.width
                val h = size.height
                drawLine(
                    color = checkColor,
                    start = Offset(w * 0.1f, h * 0.55f),
                    end = Offset(w * 0.4f, h * 0.85f),
                    strokeWidth = stroke,
                    cap = StrokeCap.Round
                )
                drawLine(
                    color = checkColor,
                    start = Offset(w * 0.4f, h * 0.85f),
                    end = Offset(w * 0.9f, h * 0.15f),
                    strokeWidth = stroke,
                    cap = StrokeCap.Round
                )
            }
        }
    }
}