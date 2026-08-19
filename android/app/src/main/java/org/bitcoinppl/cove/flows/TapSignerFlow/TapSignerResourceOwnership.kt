@file:Suppress("PackageNaming") // package name matches the existing TapSignerFlow namespace

package org.bitcoinppl.cove.flows.TapSignerFlow

import org.bitcoinppl.cove_core.types.FfiConverterTypePsbt
import org.bitcoinppl.cove_core.types.Psbt

internal fun Psbt.cloneForTapSignerCommand(): Psbt =
    FfiConverterTypePsbt.lift(uniffiCloneHandle())
