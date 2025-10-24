package org.bitcoinppl.cove.transaction_details

import android.content.Intent
import android.net.Uri
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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.components.ConfirmationIndicatorView
import org.bitcoinppl.cove_core.TransactionDetails
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.types.TxId
import org.bitcoinppl.cove_core.types.BitcoinUnit
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.ImageButton
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.tooling.preview.Preview
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.min

/**
 * transaction details screen - now using manager-based pattern
 * ported from iOS TransactionDetailsView.swift
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TransactionDetailsScreen(
    app: AppManager,
    manager: WalletManager,
    details: TransactionDetails,
) {
    val context = LocalContext.current
    val metadata = manager.walletMetadata

    // state for confirmation polling
    var numberOfConfirmations by remember { mutableStateOf<Int?>(null) }
    var transactionDetails by remember { mutableStateOf(details) }
    var feeFiatFmt by remember { mutableStateOf("---") }
    var sentSansFeeFiatFmt by remember { mutableStateOf("---") }
    var totalSpentFiatFmt by remember { mutableStateOf("---") }

    // get current color scheme
    val isDark = androidx.compose.foundation.isSystemInDarkTheme()

    // load fiat amounts
    LaunchedEffect(transactionDetails) {
        feeFiatFmt = try { transactionDetails.feeFiatFmt() } catch (e: Exception) { "---" }
        sentSansFeeFiatFmt = try { transactionDetails.sentSansFeeFiatFmt() } catch (e: Exception) { "---" }
        totalSpentFiatFmt = try { transactionDetails.amountFiatFmt() } catch (e: Exception) { "---" }
    }

    // poll for confirmations if not fully confirmed
    LaunchedEffect(transactionDetails.txId()) {
        if (!transactionDetails.isConfirmed()) {
            // start with a delay to avoid race condition
            delay(2000)
        }

        var needsFrequentCheck = true
        var errors = 0

        while (true) {
            try {
                // refresh transaction details
                val updated = manager.rust.transactionDetails(txId = transactionDetails.txId())
                if (updated != null) {
                    transactionDetails = updated
                }

                // get confirmations
                val blockNumber = transactionDetails.blockNumber()
                if (blockNumber != null) {
                    val confirmations = manager.rust.numberOfConfirmations(blockHeight = blockNumber)
                    numberOfConfirmations = confirmations.toInt()

                    // if fully confirmed, slow down polling
                    if (confirmations >= 3u && needsFrequentCheck) {
                        needsFrequentCheck = false
                    }
                }

                // wait before next poll
                if (needsFrequentCheck) {
                    delay(30000) // 30 seconds
                } else {
                    delay(60000) // 60 seconds
                }
            } catch (e: Exception) {
                android.util.Log.e("TransactionDetails", "error polling confirmations", e)
                errors++
                if (errors > 10) break
                delay(30000)
            }
        }
    }

    val snackbarHostState = remember { SnackbarHostState() }

    // theme colors
    val bg = if (isDark) Color(0xFF000000) else Color(0xFFFFFFFF)
    val fg = if (isDark) Color(0xFFEFEFEF) else Color(0xFF101010)
    val sub = if (isDark) Color(0xFFB8B8B8) else Color(0xFF8F8F95)
    val checkCircle = if (isDark) Color(0xFF0F0F12) else Color(0xFF0F1012)
    val chipBg = CoveColor.TransactionReceived

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

    // derive UI state from transaction details
    val isSent = transactionDetails.isSent()
    val isReceived = transactionDetails.isReceived()
    val isConfirmed = transactionDetails.isConfirmed()

    val headerTitle = stringResource(
        id = if (isConfirmed) {
            if (isSent) R.string.title_transaction_sent else R.string.title_transaction_received
        } else {
            R.string.title_transaction_pending
        }
    )

    val actionLabelRes = if (isSent) R.string.label_transaction_sent else R.string.label_transaction_received
    val actionIcon = if (isSent) Icons.Filled.NorthEast else Icons.Filled.SouthWest

    // format date
    val formattedDate = transactionDetails.confirmationDateTime() ?: ""
    val message = if (isConfirmed && formattedDate.isNotEmpty()) {
        stringResource(
            id = if (isSent) R.string.label_transaction_sent_on else R.string.label_transaction_received_on,
            formattedDate
        )
    } else if (!isConfirmed) {
        stringResource(R.string.label_transaction_pending)
    } else {
        ""
    }

    // format amounts
    val txAmountPrimary = manager.rust.displayAmount(amount = transactionDetails.amount())
    val txAmountSecondary by androidx.compose.runtime.produceState(initialValue = "---") {
        value = try {
            "≈ ${transactionDetails.amountFiatFmt()}"
        } catch (e: Exception) {
            "---"
        }
    }

    // details expanded from metadata
    val isExpanded = metadata?.detailsExpanded ?: false

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
                    IconButton(onClick = { app.popRoute() }) {
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

                // TODO: Add label functionality - deferred to future phase
                // Row(
                //     verticalAlignment = Alignment.CenterVertically,
                //     modifier = Modifier
                //         .clip(RoundedCornerShape(16.dp))
                //         .clickable { /* TODO */ }
                // ) {
                //     Box(
                //         modifier = Modifier
                //             .size(18.dp)
                //             .clip(CircleShape)
                //             .background(chipBg),
                //         contentAlignment = Alignment.Center
                //     ) {
                //         Icon(
                //             imageVector = Icons.Default.Add,
                //             contentDescription = null,
                //             tint = Color.White,
                //             modifier = Modifier.size(14.dp)
                //         )
                //     }
                //     Spacer(Modifier.size(8.dp))
                //     Text(stringResource(R.string.btn_add_label), color = fg, fontSize = 16.sp)
                // }

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

                // show confirmation indicator if < 3 confirmations
                if (numberOfConfirmations != null && numberOfConfirmations!! < 3) {
                    Spacer(Modifier.height(24.dp))
                    Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(1.dp)
                            .background(if (isDark) Color(0xFF222428) else Color(0xFFE4E5E7))
                    )
                    Spacer(Modifier.height(24.dp))
                    ConfirmationIndicatorView(
                        current = numberOfConfirmations!!,
                        modifier = Modifier.fillMaxWidth()
                    )
                }

                if (isExpanded) {
                    TransactionDetailsWidget(
                        manager = manager,
                        transactionDetails = transactionDetails,
                        numberOfConfirmations = numberOfConfirmations,
                        isDark = isDark,
                        feeFiatFmt = feeFiatFmt,
                        sentSansFeeFiatFmt = sentSansFeeFiatFmt,
                        totalSpentFiatFmt = totalSpentFiatFmt,
                        metadata = metadata
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
                        onClick = {
                            val url = transactionDetails.transactionUrl()
                            val intent = Intent(Intent.ACTION_VIEW, Uri.parse(url))
                            context.startActivity(intent)
                        },
                        colors = ButtonDefaults.buttonColors(
                            containerColor = if (isDark) CoveColor.SurfaceDark else CoveColor.midnightBlue,
                            contentColor = if (isDark) CoveColor.BorderLight else Color.White
                        ),
                        modifier = Modifier
                            .fillMaxWidth()
                    )

                    Spacer(Modifier.height(12.dp))

                    TextButton(
                        onClick = {
                            // TODO: implement toggle details expanded
                        },
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
    manager: WalletManager,
    transactionDetails: TransactionDetails,
    numberOfConfirmations: Int?,
    isDark: Boolean,
    feeFiatFmt: String,
    sentSansFeeFiatFmt: String,
    totalSpentFiatFmt: String,
    metadata: WalletMetadata?
) {
    val dividerColor = if (isDark) Color(0xFF222428) else Color(0xFFE4E5E7)
    val sub = if (isDark) Color(0xFFB8B8B8) else Color(0xFF8F8F95)
    val fg = if (isDark) Color(0xFFEFEFEF) else Color(0xFF101010)
    val isSent = transactionDetails.isSent()
    val isConfirmed = transactionDetails.isConfirmed()

    Spacer(Modifier.height(48.dp))
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .height(1.dp)
            .background(dividerColor)
    )
    Spacer(Modifier.height(24.dp))

    // show confirmations if confirmed
    if (isConfirmed) {
        Column(modifier = Modifier.fillMaxWidth()) {
            Text(
                "Confirmations",
                color = if (isDark) Color(0xFFB8B8B8) else Color(0xFF6F6F75),
                fontSize = 14.sp
            )
            Spacer(Modifier.height(8.dp))
            if (numberOfConfirmations != null) {
                Text(
                    numberOfConfirmations.toString(),
                    color = fg,
                    fontSize = 16.sp,
                    fontWeight = FontWeight.SemiBold
                )
            }
            Spacer(Modifier.height(14.dp))

            Text(
                "Block Number",
                color = if (isDark) Color(0xFFB8B8B8) else Color(0xFF6F6F75),
                fontSize = 14.sp
            )
            Spacer(Modifier.height(8.dp))
            Text(
                transactionDetails.blockNumberFmt() ?: "",
                color = fg,
                fontSize = 16.sp,
                fontWeight = FontWeight.SemiBold
            )
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

    // address (sent to / received from)
    val addressLabel = stringResource(
        if (isSent) R.string.label_sent_to else R.string.label_received_from
    )
    Column(modifier = Modifier.fillMaxWidth()) {
        Text(
            addressLabel,
            color = if (isDark) Color(0xFFB8B8B8) else Color(0xFF6F6F75),
            fontSize = 16.sp
        )
        Spacer(Modifier.height(8.dp))
        Text(
            transactionDetails.addressSpacedOut(),
            color = fg,
            fontSize = 20.sp,
            fontWeight = FontWeight.SemiBold,
            lineHeight = 24.sp
        )

        // show block number and confirmations for confirmed sent transactions
        if (isSent && isConfirmed) {
            Spacer(Modifier.height(8.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    transactionDetails.blockNumberFmt() ?: "",
                    color = sub,
                    fontSize = 14.sp
                )
                Text(" | ", color = sub, fontSize = 14.sp)
                if (numberOfConfirmations != null) {
                    Text(
                        numberOfConfirmations.toString(),
                        color = sub,
                        fontSize = 14.sp
                    )
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
    }
    Spacer(Modifier.height(24.dp))
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .height(1.dp)
            .background(dividerColor)
    )
    Spacer(Modifier.height(24.dp))

    // network fee (for sent transactions)
    if (isSent) {
        DetailsWidget(
            label = stringResource(R.string.label_network_fee),
            primary = transactionDetails.feeFmt(unit = metadata?.selectedUnit ?: BitcoinUnit.SAT),
            secondary = "≈ $feeFiatFmt",
            isDark = isDark,
            showInfoIcon = true,
            onInfoClick = { /* TODO: show fee info */ }
        )
        Spacer(Modifier.height(24.dp))

        DetailsWidget(
            label = stringResource(R.string.label_recipient_receives),
            primary = transactionDetails.sentSansFeeFmt(unit = metadata?.selectedUnit ?: BitcoinUnit.SAT),
            secondary = "≈ $sentSansFeeFiatFmt",
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
            primary = transactionDetails.amountFmt(unit = metadata?.selectedUnit ?: BitcoinUnit.SAT),
            secondary = "≈ $totalSpentFiatFmt",
            isDark = isDark,
            isTotal = true
        )
    }

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