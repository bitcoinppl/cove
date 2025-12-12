package org.bitcoinppl.cove.transaction_details

import android.content.Intent
import android.net.Uri
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.NorthEast
import androidx.compose.material.icons.filled.SouthWest
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.components.ConfirmationIndicatorView
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.ui.theme.isLight
import org.bitcoinppl.cove.utils.toColor
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove.views.BalanceAutoSizeText
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove_core.HeaderIconPresenter
import org.bitcoinppl.cove_core.TransactionDetails
import org.bitcoinppl.cove_core.TransactionState
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.types.FfiColorScheme
import org.bitcoinppl.cove_core.types.TransactionDirection

private const val INITIAL_DELAY_MS = 2000L
private const val FREQUENT_POLL_INTERVAL_MS = 30000L
private const val NORMAL_POLL_INTERVAL_MS = 60000L
private const val MAX_POLL_ERRORS = 10
private const val CONFIRMATIONS_THRESHOLD = 3

/**
 * Transaction details screen - now using manager-based pattern
 * Ported from iOS TransactionDetailsView.swift
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TransactionDetailsScreen(
    app: AppManager,
    manager: WalletManager,
    details: TransactionDetails,
) {
    val context = LocalContext.current
    val metadata = manager.walletMetadata ?: return
    val scope = rememberCoroutineScope()
    val txId = details.txId()

    // read transaction details from cache (observable), fallback to passed-in details
    val transactionDetails = manager.transactionDetailsCache[txId] ?: details

    // state for confirmation polling and pull-to-refresh
    var numberOfConfirmations by remember { mutableStateOf<Int?>(null) }
    var isRefreshing by remember { mutableStateOf(false) }
    var feeFiatFmt by remember { mutableStateOf("---") }
    var sentSansFeeFiatFmt by remember { mutableStateOf("---") }
    var totalSpentFiatFmt by remember { mutableStateOf("---") }

    // get current color scheme (respects in-app theme toggle)
    val isDark = !MaterialTheme.colorScheme.isLight

    // immediately fetch fresh transaction details on screen load
    LaunchedEffect(Unit) {
        try {
            val freshDetails = manager.rust.transactionDetails(txId = txId)
            manager.updateTransactionDetailsCache(txId, freshDetails)
        } catch (e: Exception) {
            android.util.Log.e("TransactionDetails", "error fetching fresh details", e)
        }
    }

    // load fiat amounts
    LaunchedEffect(transactionDetails) {
        feeFiatFmt =
            try {
                transactionDetails.feeFiatFmt()
            } catch (e: Exception) {
                "---"
            }
        sentSansFeeFiatFmt =
            try {
                transactionDetails.sentSansFeeFiatFmt()
            } catch (e: Exception) {
                "---"
            }
        totalSpentFiatFmt =
            try {
                transactionDetails.amountFiatFmt()
            } catch (e: Exception) {
                "---"
            }
    }

    // poll for confirmations if not fully confirmed
    LaunchedEffect(txId) {
        if (!transactionDetails.isConfirmed()) {
            delay(INITIAL_DELAY_MS)
        }

        var needsFrequentCheck = true
        var errors = 0

        while (isActive) {
            try {
                ensureActive()

                // refresh transaction details and update cache
                val freshDetails = manager.rust.transactionDetails(txId = txId)
                if (!isActive) break
                manager.updateTransactionDetailsCache(txId, freshDetails)

                // get confirmations from fresh details
                val blockNumber = freshDetails.blockNumber()
                if (blockNumber != null) {
                    val confirmations = manager.rust.numberOfConfirmations(blockHeight = blockNumber)
                    if (!isActive) break
                    numberOfConfirmations = confirmations.toInt()

                    // if fully confirmed, slow down polling
                    if (confirmations >= CONFIRMATIONS_THRESHOLD.toUInt() && needsFrequentCheck) {
                        needsFrequentCheck = false
                    }
                }

                // reset error count on success
                errors = 0

                // wait before next poll
                if (needsFrequentCheck) {
                    delay(FREQUENT_POLL_INTERVAL_MS)
                } else {
                    delay(NORMAL_POLL_INTERVAL_MS)
                }
            } catch (e: CancellationException) {
                // composable left composition, exit gracefully
                break
            } catch (e: Exception) {
                android.util.Log.e("TransactionDetails", "error polling confirmations", e)
                errors++
                if (errors > MAX_POLL_ERRORS) {
                    break
                }
                delay(FREQUENT_POLL_INTERVAL_MS)
            }
        }
    }

    val snackbarHostState = remember { SnackbarHostState() }

    // theme colors
    val bg = MaterialTheme.colorScheme.background
    val fg = MaterialTheme.colorScheme.onBackground
    val sub = MaterialTheme.colorScheme.onSurfaceVariant

    // derive UI state from transaction details
    val isSent = transactionDetails.isSent()
    val isReceived = transactionDetails.isReceived()
    val isConfirmed = transactionDetails.isConfirmed()

    // header icon presenter for dynamic colors
    val presenter = remember { HeaderIconPresenter() }
    val txState = if (isConfirmed) TransactionState.CONFIRMED else TransactionState.PENDING
    val direction = if (isSent) TransactionDirection.OUTGOING else TransactionDirection.INCOMING
    val colorScheme = if (isDark) FfiColorScheme.DARK else FfiColorScheme.LIGHT
    val confirmationCount = numberOfConfirmations?.toLong() ?: if (isConfirmed) 5L else 0L

    // get colors from presenter (matching iOS)
    val circleColor = presenter.backgroundColor(txState, direction, colorScheme, confirmationCount).toColor()
    val iconColor = presenter.iconColor(txState, direction, colorScheme, confirmationCount).toColor()

    // get ring colors (matching iOS opacities)
    val ringOpacities = if (isDark) listOf(0.88f, 0.66f, 0.33f) else listOf(0.44f, 0.24f, 0.10f)
    val ringColors: List<Color> =
        listOf(
            presenter
                .ringColor(txState, colorScheme, direction, confirmationCount, 1L)
                .toColor()
                .let { color -> color.copy(alpha = color.alpha * ringOpacities[0]) },
            presenter
                .ringColor(txState, colorScheme, direction, confirmationCount, 2L)
                .toColor()
                .let { color -> color.copy(alpha = color.alpha * ringOpacities[1]) },
            presenter
                .ringColor(txState, colorScheme, direction, confirmationCount, 3L)
                .toColor()
                .let { color -> color.copy(alpha = color.alpha * ringOpacities[2]) },
        )

    val headerTitle =
        stringResource(
            id =
                if (isConfirmed) {
                    if (isSent) R.string.title_transaction_sent else R.string.title_transaction_received
                } else {
                    R.string.title_transaction_pending
                },
        )

    val actionLabelRes =
        when {
            isConfirmed && isSent -> R.string.label_transaction_sent
            isConfirmed && !isSent -> R.string.label_transaction_received
            !isConfirmed && isSent -> R.string.label_transaction_sending
            else -> R.string.label_transaction_receiving
        }
    val actionIcon = if (isSent) Icons.Filled.NorthEast else Icons.Filled.SouthWest

    // format date
    val formattedDate = transactionDetails.confirmationDateTime() ?: ""

    // format amounts
    val txAmountPrimary = manager.rust.displayAmount(amount = transactionDetails.amount())
    val txAmountSecondary by androidx.compose.runtime.produceState(initialValue = "---") {
        value =
            try {
                transactionDetails.amountFiatFmt()
            } catch (e: Exception) {
                "---"
            }
    }

    // details expanded from metadata
    val isExpanded = metadata.detailsExpanded

    // get capsule config for transaction status
    val systemGreen = if (isDark) CoveColor.SystemGreenDark else CoveColor.SystemGreenLight
    val capsuleConfig =
        when {
            isReceived && isConfirmed -> {
                Triple(
                    systemGreen.copy(alpha = 0.2f),
                    systemGreen,
                    false,
                )
            }
            isSent && isConfirmed -> {
                Triple(
                    Color.Black,
                    Color.White,
                    true,
                )
            }
            else -> {
                Triple(
                    CoveColor.coolGray,
                    Color.Black.copy(alpha = 0.8f),
                    false,
                )
            }
        }

    Scaffold(
        containerColor = bg,
        topBar = {
            CenterAlignedTopAppBar(
                colors =
                    TopAppBarDefaults.centerAlignedTopAppBarColors(
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
                            contentDescription = null,
                        )
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
        // parallax + fade effect on background
        val scrollState = rememberScrollState()
        val scrollOffset = scrollState.value.toFloat()
        val parallaxOffset = (-60).dp - (scrollOffset * 0.3f).dp
        val fadeAlpha = (1f - (scrollOffset / 275f)).coerceIn(0f, if (isDark) 1.0f else 0.40f)

        PullToRefreshBox(
            isRefreshing = isRefreshing,
            onRefresh = {
                scope.launch {
                    isRefreshing = true
                    try {
                        val freshDetails = manager.rust.transactionDetails(txId = txId)
                        manager.updateTransactionDetailsCache(txId, freshDetails)

                        // also update confirmations
                        val blockNumber = freshDetails.blockNumber()
                        if (blockNumber != null) {
                            val confirmations = manager.rust.numberOfConfirmations(blockHeight = blockNumber)
                            numberOfConfirmations = confirmations.toInt()
                        }
                    } catch (e: Exception) {
                        android.util.Log.e("TransactionDetails", "error refreshing details", e)
                    }
                    isRefreshing = false
                }
            },
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(bottom = padding.calculateBottomPadding()),
        ) {
            BoxWithConstraints(
                modifier = Modifier.fillMaxSize(),
            ) {
                val minHeight = maxHeight

                Image(
                    painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    modifier =
                        Modifier
                            .fillMaxHeight()
                            .align(Alignment.TopCenter)
                            .offset(y = parallaxOffset)
                            .alpha(fadeAlpha),
                )

                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .defaultMinSize(minHeight = minHeight)
                            .padding(horizontal = 20.dp)
                            .verticalScroll(scrollState),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    // add top padding to account for top bar
                    Spacer(Modifier.height(padding.calculateTopPadding() + 16.dp))

                    val configuration = LocalConfiguration.current
                    val headerSize = (configuration.screenWidthDp * 0.33f).dp

                    CheckWithRingsWidget(
                        diameter = headerSize,
                        circleColor = circleColor,
                        ringColors = ringColors,
                        iconColor = iconColor,
                        isConfirmed = isConfirmed,
                    )

                    Spacer(Modifier.height(16.dp))

                    Text(
                        headerTitle,
                        color = fg,
                        fontSize = 28.sp,
                        fontWeight = FontWeight.SemiBold,
                        lineHeight = 32.sp,
                    )

                    Spacer(Modifier.height(4.dp))

                    TransactionLabelView(
                        transactionDetails = transactionDetails,
                        manager = manager,
                        secondaryColor = sub,
                        snackbarHostState = snackbarHostState,
                    )

                    Spacer(Modifier.height(24.dp))

                    // show status message with date
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        if (isConfirmed && formattedDate.isNotEmpty()) {
                            Text(
                                text =
                                    if (isSent) {
                                        "Your transaction was sent on"
                                    } else {
                                        "Your transaction was successfully received"
                                    },
                                color = sub,
                                fontSize = 16.sp,
                                textAlign = TextAlign.Center,
                                lineHeight = 22.sp,
                            )
                            Text(
                                text = formattedDate,
                                color = sub,
                                fontSize = 16.sp,
                                fontWeight = FontWeight.Medium,
                                textAlign = TextAlign.Center,
                                lineHeight = 22.sp,
                            )
                        } else if (!isConfirmed) {
                            Text(
                                text = "Your transaction is pending.",
                                color = sub,
                                fontSize = 16.sp,
                                textAlign = TextAlign.Center,
                                lineHeight = 22.sp,
                            )
                            Text(
                                text = "Please check back soon for an update.",
                                color = sub,
                                fontSize = 16.sp,
                                fontWeight = FontWeight.SemiBold,
                                textAlign = TextAlign.Center,
                                lineHeight = 22.sp,
                            )
                        }
                    }

                    Spacer(Modifier.height(32.dp))

                    BalanceAutoSizeText(
                        txAmountPrimary,
                        color = fg,
                        baseFontSize = 36.sp,
                        minimumScaleFactor = 0.01f,
                        fontWeight = FontWeight.Bold,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth(),
                    )

                    Spacer(Modifier.height(4.dp))

                    AutoSizeText(
                        txAmountSecondary,
                        color = fg.copy(alpha = 0.8f),
                        maxFontSize = 18.sp,
                        minimumScaleFactor = 0.90f,
                    )

                    Spacer(Modifier.height(32.dp))

                    TransactionCapsule(
                        text = stringResource(actionLabelRes),
                        icon = actionIcon,
                        backgroundColor = capsuleConfig.first,
                        textColor = capsuleConfig.second,
                        showStroke = capsuleConfig.third,
                    )

                    Spacer(Modifier.height(32.dp))

                    // show confirmation indicator if < 3 confirmations
                    if (numberOfConfirmations != null && numberOfConfirmations!! < 3) {
                        Column(modifier = Modifier.fillMaxWidth()) {
                            Spacer(Modifier.height(24.dp))
                            Box(
                                modifier =
                                    Modifier
                                        .fillMaxWidth()
                                        .height(1.dp)
                                        .background(MaterialTheme.colorScheme.outlineVariant),
                            )
                            Spacer(Modifier.height(24.dp))
                            ConfirmationIndicatorView(
                                current = numberOfConfirmations!!,
                                modifier = Modifier.fillMaxWidth(),
                            )
                            // only add spacing if details are collapsed
                            if (!isExpanded) {
                                Spacer(Modifier.height(32.dp))
                            }
                        }
                    }

                    AnimatedVisibility(
                        visible = isExpanded,
                        enter = expandVertically() + fadeIn(),
                        exit = shrinkVertically() + fadeOut(),
                    ) {
                        TransactionDetailsWidget(
                            transactionDetails = transactionDetails,
                            numberOfConfirmations = numberOfConfirmations,
                            feeFiatFmt = feeFiatFmt,
                            sentSansFeeFiatFmt = sentSansFeeFiatFmt,
                            totalSpentFiatFmt = totalSpentFiatFmt,
                            metadata = metadata,
                        )
                    }

                    // flexible spacer to push buttons to bottom (matches iOS Spacer() behavior)
                    Spacer(Modifier.weight(1f))

                    Column(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .padding(bottom = 16.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        ImageButton(
                            text = stringResource(R.string.btn_view_in_explorer),
                            onClick = {
                                val url = transactionDetails.transactionUrl()
                                val intent = Intent(Intent.ACTION_VIEW, Uri.parse(url))
                                context.startActivity(intent)
                            },
                            colors =
                                ButtonDefaults.buttonColors(
                                    containerColor = MaterialTheme.coveColors.midnightBtn,
                                    contentColor = Color.White,
                                ),
                            modifier =
                                Modifier
                                    .fillMaxWidth(),
                        )

                        Spacer(Modifier.height(16.dp))

                        TextButton(
                            onClick = {
                                manager.dispatch(WalletManagerAction.ToggleDetailsExpanded)
                            },
                            modifier =
                                Modifier
                                    .align(Alignment.CenterHorizontally)
                                    .offset(y = (-20).dp),
                        ) {
                            Text(
                                text = stringResource(if (isExpanded) R.string.btn_hide_details else R.string.btn_show_details),
                                color = sub.copy(alpha = 0.8f),
                                fontSize = 13.sp,
                                textAlign = TextAlign.Center,
                                fontWeight = FontWeight.Bold,
                            )
                        }
                    }
                }
            }
        }
    }
}
