package org.bitcoinppl.cove.utils

import org.bitcoinppl.cove_core.BootstrapStep
import org.bitcoinppl.cove_core.bootstrapStepIsMigrationInProgress

val BootstrapStep.isMigrationInProgress: Boolean
    get() = bootstrapStepIsMigrationInProgress(this)
