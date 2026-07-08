package org.bitcoinppl.cove.sidebar

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SidebarDragStateTest {
    @Test
    fun openDragFromClosedIsNotResetByUnchangedClosedVisibility() {
        val state = SidebarDragState(initialSidebarVisible = false)

        state.startDrag(
            offsetX = 32f,
            edgeThresholdPx = 50f,
            sidebarVisible = false,
        )
        state.dragBy(
            dragAmount = 120f,
            targetOffsetPx = 0f,
            sidebarWidthPx = SIDEBAR_WIDTH,
        )

        state.reconcileSidebarVisibility(sidebarVisible = false)

        assertTrue(state.isDragging)
        assertTrue(state.isValidDrag)
        assertEquals(120f, state.currentOffset(0f, 0f, SIDEBAR_WIDTH), 0.001f)
    }

    @Test
    fun closeTransitionResetsActiveDrag() {
        val state = SidebarDragState(initialSidebarVisible = true)

        state.startDrag(
            offsetX = 200f,
            edgeThresholdPx = 50f,
            sidebarVisible = true,
        )
        state.dragBy(
            dragAmount = -100f,
            targetOffsetPx = SIDEBAR_WIDTH,
            sidebarWidthPx = SIDEBAR_WIDTH,
        )

        state.reconcileSidebarVisibility(sidebarVisible = false)

        assertFalse(state.isDragging)
        assertFalse(state.isValidDrag)
        assertEquals(0f, state.currentOffset(0f, 0f, SIDEBAR_WIDTH), 0.001f)
    }

    @Test
    fun cancelledOpenDragSettlesClosedFromCurrentFingerPosition() {
        val state = SidebarDragState(initialSidebarVisible = false)

        state.startDrag(
            offsetX = 32f,
            edgeThresholdPx = 50f,
            sidebarVisible = false,
        )
        state.dragBy(
            dragAmount = 120f,
            targetOffsetPx = 0f,
            sidebarWidthPx = SIDEBAR_WIDTH,
        )

        val settle = state.cancelDrag(
            targetOffsetPx = 0f,
            sidebarWidthPx = SIDEBAR_WIDTH,
        )

        assertEquals(SidebarDragCancel(currentOffsetPx = 120f, targetOffsetPx = 0f), settle)
        assertFalse(state.isDragging)
        assertFalse(state.isValidDrag)
    }

    @Test
    fun openDragPastThresholdSettlesOpen() {
        val state = SidebarDragState(initialSidebarVisible = false)

        state.startDrag(
            offsetX = 32f,
            edgeThresholdPx = 50f,
            sidebarVisible = false,
        )
        state.dragBy(
            dragAmount = 120f,
            targetOffsetPx = 0f,
            sidebarWidthPx = SIDEBAR_WIDTH,
        )

        val settle = state.endDrag(
            targetOffsetPx = 0f,
            sidebarWidthPx = SIDEBAR_WIDTH,
        )

        assertEquals(
            SidebarDragEnd(
                currentOffsetPx = 120f,
                targetOffsetPx = SIDEBAR_WIDTH,
                sidebarVisible = true,
            ),
            settle,
        )
    }

    @Test
    fun dragUsesFreshTargetWhenSidebarVisibilityChanges() {
        val state = SidebarDragState(initialSidebarVisible = true)

        state.startDrag(
            offsetX = 200f,
            edgeThresholdPx = 50f,
            sidebarVisible = true,
        )
        state.dragBy(
            dragAmount = -60f,
            targetOffsetPx = SIDEBAR_WIDTH,
            sidebarWidthPx = SIDEBAR_WIDTH,
        )

        state.dragBy(
            dragAmount = -400f,
            targetOffsetPx = 0f,
            sidebarWidthPx = SIDEBAR_WIDTH,
        )

        assertEquals(0f, state.currentOffset(0f, 0f, SIDEBAR_WIDTH), 0.001f)
    }

    @Test
    fun invalidDragDoesNotSettle() {
        val state = SidebarDragState(initialSidebarVisible = false)

        state.startDrag(
            offsetX = 80f,
            edgeThresholdPx = 50f,
            sidebarVisible = false,
        )

        assertNull(state.endDrag(targetOffsetPx = 0f, sidebarWidthPx = SIDEBAR_WIDTH))
        assertFalse(state.isDragging)
        assertFalse(state.isValidDrag)
    }

    private companion object {
        const val SIDEBAR_WIDTH = 280f
    }
}
