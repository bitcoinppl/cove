package org.bitcoinppl.cove.views

import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TorStatus

/**
 * Bootstrap detail for display, falling back to the launch copy until the runtime reports its
 * first progress line
 */
@Composable
fun torBootstrapMessage(status: TorStatus.Bootstrapping): String =
    status.message ?: stringResource(R.string.tor_status_starting)
