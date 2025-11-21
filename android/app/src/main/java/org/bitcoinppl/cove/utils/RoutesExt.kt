package org.bitcoinppl.cove.utils

import org.bitcoinppl.cove_core.HotWalletRoute
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory

// Extension functions for routes to provide cleaner syntax
// Ported from iOS Routes+Ext.swift

/**
 * Convert a HotWalletRoute into a full Route
 */
fun HotWalletRoute.intoRoute(): Route = RouteFactory().hotWallet(this)
