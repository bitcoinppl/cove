@file:Suppress("PackageNaming") // package name matches the existing TapSignerFlow namespace

package org.bitcoinppl.cove.flows.TapSignerFlow

import org.bitcoinppl.cove_core.FfiConverterTypeSetupCmd
import org.bitcoinppl.cove_core.FfiConverterTypeTapSignerCvc
import org.bitcoinppl.cove_core.FfiConverterTypeTapSignerSetupContinuation
import org.bitcoinppl.cove_core.SetupCmd
import org.bitcoinppl.cove_core.TapSignerCvc
import org.bitcoinppl.cove_core.TapSignerSetupContinuation
import org.bitcoinppl.cove_core.types.FfiConverterTypePsbt
import org.bitcoinppl.cove_core.types.Psbt

internal fun SetupCmd.cloneForTapSignerCommand(): SetupCmd =
    FfiConverterTypeSetupCmd.lift(uniffiCloneHandle())

internal fun TapSignerCvc.cloneForTapSignerCommand(): TapSignerCvc =
    FfiConverterTypeTapSignerCvc.lift(uniffiCloneHandle())

internal fun TapSignerSetupContinuation.cloneForTapSignerCommand(): TapSignerSetupContinuation =
    FfiConverterTypeTapSignerSetupContinuation.lift(uniffiCloneHandle())

internal fun Psbt.cloneForTapSignerCommand(): Psbt =
    FfiConverterTypePsbt.lift(uniffiCloneHandle())
