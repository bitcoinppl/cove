package org.bitcoinppl.cove.sidebar

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Nfc
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.zIndex
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.WalletColor
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.types.WalletId

private const val TAG = "SidebarView"
private const val DRAG_SCALE = 1.02f
private const val DRAG_SHADOW_ELEVATION_DP = 12f
private const val INTER_ITEM_SPACING_DP = 12

@Composable
fun SidebarView(
    app: AppManager,
    modifier: Modifier = Modifier,
) {
    val density = LocalDensity.current
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val spacingPx = with(density) { INTER_ITEM_SPACING_DP.dp.toPx() }

    // local copy lets drags apply optimistically; resyncs from app.wallets when idle
    var walletList by remember { mutableStateOf(app.wallets) }
    var draggedWalletId by remember { mutableStateOf<WalletId?>(null) }
    var draggedOffsetY by remember { mutableFloatStateOf(0f) }
    var itemHeightPx by remember { mutableFloatStateOf(0f) }

    LaunchedEffect(app.wallets) {
        if (draggedWalletId == null) walletList = app.wallets
    }

    Column(
        modifier =
            modifier
                .width(280.dp)
                .fillMaxHeight()
                .background(CoveColor.midnightBlue)
                .padding(WindowInsets.safeDrawing.asPaddingValues())
                .padding(horizontal = 20.dp),
        verticalArrangement = Arrangement.spacedBy(0.dp),
    ) {
        // header with icon and NFC button
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Image(
                painter = painterResource(id = R.drawable.cove_logo),
                contentDescription = "Cove",
                modifier =
                    Modifier
                        .size(65.dp)
                        .clip(CircleShape),
            )

            IconButton(
                onClick = {
                    app.closeSidebarAndNavigate {
                        app.scanNfc()
                    }
                },
            ) {
                Icon(
                    imageVector = Icons.Default.Nfc,
                    contentDescription = "NFC Scan",
                    tint = Color.White,
                    modifier = Modifier.size(24.dp),
                )
            }
        }

        Spacer(modifier = Modifier.height(22.dp))

        HorizontalDivider(
            color = Color.White.copy(alpha = 0.5f),
            thickness = 1.dp,
        )

        Spacer(modifier = Modifier.height(22.dp))

        // my wallets header
        Text(
            text = "My Wallets",
            color = Color.White,
            fontSize = 17.sp,
            fontWeight = FontWeight.Medium,
        )

        Spacer(modifier = Modifier.height(12.dp))

        // wallet list
        LazyColumn(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(INTER_ITEM_SPACING_DP.dp),
        ) {
            items(walletList, key = { it.id }) { wallet ->
                val isDragged = wallet.id == draggedWalletId
                val isAnyDragging = draggedWalletId != null

                // only neighbors slide smoothly; the dragged item tracks the finger via
                // graphicsLayer and should not be animated by the lazy list
                val placementModifier = if (isDragged) Modifier else Modifier.animateItem()

                WalletItem(
                    wallet = wallet,
                    isDragged = isDragged,
                    dragOffsetY = if (isDragged) draggedOffsetY else 0f,
                    isClickEnabled = !isAnyDragging,
                    placementModifier = placementModifier,
                    onClick = {
                        app.closeSidebarAndNavigate {
                            app.rust.selectWallet(wallet.id)
                        }
                    },
                    onMeasuredHeight = { h ->
                        if (itemHeightPx == 0f) itemHeightPx = h.toFloat()
                    },
                    gestureModifier =
                        Modifier.pointerInput(wallet.id) {
                            detectDragGesturesAfterLongPress(
                                onDragStart = {
                                    haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                                    draggedWalletId = wallet.id
                                    draggedOffsetY = 0f
                                },
                                onDrag = onDrag@{ change, dragAmount ->
                                    change.consume()
                                    val draggingId = draggedWalletId ?: return@onDrag
                                    val step = itemHeightPx + spacingPx
                                    if (step <= 0f) return@onDrag

                                    draggedOffsetY += dragAmount.y
                                    val fromIndex = walletList.indexOfFirst { it.id == draggingId }
                                    if (fromIndex == -1) return@onDrag

                                    val threshold = step / 2f
                                    if (draggedOffsetY > threshold && fromIndex < walletList.size - 1) {
                                        walletList =
                                            walletList.toMutableList().apply {
                                                val item = removeAt(fromIndex)
                                                add(fromIndex + 1, item)
                                            }
                                        draggedOffsetY -= step
                                    } else if (draggedOffsetY < -threshold && fromIndex > 0) {
                                        walletList =
                                            walletList.toMutableList().apply {
                                                val item = removeAt(fromIndex)
                                                add(fromIndex - 1, item)
                                            }
                                        draggedOffsetY += step
                                    }
                                },
                                onDragEnd = {
                                    val orderedIds = walletList.map { it.id }
                                    val previous = app.wallets
                                    persistReorder(
                                        scope = scope,
                                        app = app,
                                        orderedIds = orderedIds,
                                        previous = previous,
                                        haptics = haptics,
                                        onRollback = { walletList = previous },
                                    )
                                    draggedWalletId = null
                                    draggedOffsetY = 0f
                                },
                                onDragCancel = {
                                    walletList = app.wallets
                                    draggedWalletId = null
                                    draggedOffsetY = 0f
                                },
                            )
                        },
                )
            }
        }

        Spacer(modifier = Modifier.height(16.dp))

        HorizontalDivider(
            color = CoveColor.coveLightGray.copy(alpha = 0.5f),
            thickness = 1.dp,
        )

        Spacer(modifier = Modifier.height(32.dp))

        // add wallet button
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable {
                        app.closeSidebarAndNavigate {
                            if (app.wallets.isEmpty()) {
                                app.resetRoute(RouteFactory().newWalletSelect())
                            } else {
                                app.pushRoute(RouteFactory().newWalletSelect())
                            }
                        }
                    }.padding(vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(20.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = Icons.Default.Add,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(24.dp),
            )
            Text(
                text = "Add Wallet",
                color = Color.White,
                fontSize = 17.sp,
            )
        }

        Spacer(modifier = Modifier.height(22.dp))

        // settings button
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable {
                        app.closeSidebarAndNavigate {
                            app.pushRoute(Route.Settings(SettingsRoute.Main))
                        }
                    }.padding(vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(20.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = Icons.Default.Settings,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(24.dp),
            )
            Text(
                text = "Settings",
                color = Color.White,
                fontSize = 17.sp,
            )
        }
    }
}

private fun persistReorder(
    scope: CoroutineScope,
    app: AppManager,
    orderedIds: List<WalletId>,
    previous: List<WalletMetadata>,
    haptics: HapticFeedback,
    onRollback: () -> Unit,
) {
    scope.launch {
        val result =
            withContext(Dispatchers.IO) {
                runCatching { app.database.wallets().reorderWallets(orderedIds) }
            }
        result.onSuccess {
            haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
        }.onFailure { error ->
            Log.e(TAG, "Failed to reorder wallets: ${error.message}")
            onRollback()
        }
    }
}

@Composable
private fun WalletItem(
    wallet: WalletMetadata,
    isDragged: Boolean,
    dragOffsetY: Float,
    isClickEnabled: Boolean,
    onClick: () -> Unit,
    onMeasuredHeight: (Int) -> Unit,
    gestureModifier: Modifier,
    placementModifier: Modifier = Modifier,
) {
    val density = LocalDensity.current
    val shadowPx = with(density) { DRAG_SHADOW_ELEVATION_DP.dp.toPx() }

    var base: Modifier =
        Modifier
            .fillMaxWidth()
            .then(placementModifier)
            .onSizeChanged { onMeasuredHeight(it.height) }

    if (isDragged) {
        base =
            base
                .zIndex(1f)
                .graphicsLayer {
                    translationY = dragOffsetY
                    scaleX = DRAG_SCALE
                    scaleY = DRAG_SCALE
                    shadowElevation = shadowPx
                    clip = false
                }
    }

    Row(
        modifier =
            base
                .clip(RoundedCornerShape(10.dp))
                .background(CoveColor.coveLightGray.copy(alpha = 0.06f))
                .then(gestureModifier)
                .clickable(enabled = isClickEnabled, onClick = onClick)
                .padding(16.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // color indicator
        Box(
            modifier =
                Modifier
                    .size(8.dp)
                    .clip(CircleShape)
                    .background(wallet.color.toComposeColor()),
        )

        // wallet name
        AutoSizeText(
            text = wallet.name ?: "Wallet",
            color = Color.White,
            maxFontSize = 17.sp,
            minimumScaleFactor = 0.80f,
            modifier = Modifier.weight(1f),
        )
    }
}

// convert wallet color to compose color
private fun WalletColor.toComposeColor(): Color =
    when (this) {
        is WalletColor.Red -> CoveColor.pastelRed
        is WalletColor.Blue -> CoveColor.pastelBlue
        is WalletColor.Green -> CoveColor.walletGreen
        is WalletColor.Yellow -> CoveColor.pastelYellow
        is WalletColor.Orange -> CoveColor.beige
        is WalletColor.Purple -> CoveColor.walletColorPurple
        is WalletColor.Pink -> CoveColor.walletColorLightRed
        is WalletColor.CoolGray -> CoveColor.almostGray
        is WalletColor.WBeige -> CoveColor.beige
        is WalletColor.WPastelBlue -> CoveColor.pastelBlue
        is WalletColor.WPastelNavy -> CoveColor.pastelNavy
        is WalletColor.WPastelRed -> CoveColor.pastelRed
        is WalletColor.WPastelYellow -> CoveColor.pastelYellow
        is WalletColor.WLightMint -> CoveColor.lightMint
        is WalletColor.WPastelTeal -> CoveColor.pastelTeal
        is WalletColor.WLightPastelYellow -> CoveColor.lightPastelYellow
        is WalletColor.WAlmostGray -> CoveColor.almostGray
        is WalletColor.WAlmostWhite -> CoveColor.almostWhite
        is WalletColor.Custom -> Color.Gray
    }
