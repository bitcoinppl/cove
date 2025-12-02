package org.bitcoinppl.cove

import android.graphics.Bitmap
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel

object QrCodeGenerator {
    /**
     * Generate a QR code bitmap from text
     *
     * @param text The text to encode
     * @param size The size in pixels (QR codes are square)
     * @param errorCorrectionLevel Error correction level (L=7%, M=15%, Q=25%, H=30%)
     * @return Bitmap containing the QR code
     */
    fun generate(
        text: String,
        size: Int = 512,
        errorCorrectionLevel: ErrorCorrectionLevel = ErrorCorrectionLevel.L,
    ): Bitmap {
        val hints = mapOf(
            EncodeHintType.ERROR_CORRECTION to errorCorrectionLevel,
            EncodeHintType.MARGIN to 0,
        )

        val writer = QRCodeWriter()
        val bitMatrix = writer.encode(text, BarcodeFormat.QR_CODE, size, size, hints)

        val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.RGB_565)

        for (x in 0 until size) {
            for (y in 0 until size) {
                bitmap.setPixel(
                    x,
                    y,
                    if (bitMatrix[x, y]) android.graphics.Color.BLACK else android.graphics.Color.WHITE,
                )
            }
        }

        return bitmap
    }
}
