package org.bitcoinppl.cove.views

/** The background a QR sheet draws behind its code */
enum class QrSheetBackground {
    /**
     * Light whatever the app's appearance, for a code drawn straight onto the sheet, since cheap
     * scanner cameras, such as SeedSigner's, fail to find one on a dark sheet
     */
    AlwaysLight,

    /**
     * The app's own appearance, for a sheet whose code sits on a [QrCodeCard], whose white quiet
     * zone lets a camera find the code on a dark sheet
     */
    AppAppearance,
}
