package org.bitcoinppl.cove.cloudbackup

import java.io.ByteArrayOutputStream
import java.net.HttpURLConnection
import java.net.URL
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.uitest.UiTestSeams
import org.json.JSONObject

@Suppress("InjectDispatcher")
internal class DriveHttpClient(
    val endpoints: DriveApiEndpoints = DriveApiEndpoints(),
    private val ioDispatcher: CoroutineDispatcher = Dispatchers.IO,
) {
    suspend fun driveRequest(
        token: String,
        method: String,
        url: String,
        body: ByteArray? = null,
        contentType: String? = null,
    ): DriveResponse =
        withContext(ioDispatcher) {
            // every Drive request comes through here, so a debug test that restores a real
            // account's backup can't change it. An empty body is enough for uploads and deletes,
            // which ignore it; creating a folder reads the new folder's id, so it fails instead
            if (method != "GET" && UiTestSeams.driveReadOnly) {
                logDriveWarning("google drive request skipped, drive is read-only method=$method url=$url")
                return@withContext DriveResponse(HTTP_SUCCESS_MIN, ByteArray(0))
            }

            val connection = (URL(url).openConnection() as HttpURLConnection)
            try {
                connection.requestMethod = method
                connection.connectTimeout = NETWORK_TIMEOUT_MS
                connection.readTimeout = NETWORK_TIMEOUT_MS
                connection.setRequestProperty("Authorization", "Bearer $token")
                connection.setRequestProperty("Accept", "application/json")

                if (body != null) {
                    connection.doOutput = true
                    connection.setRequestProperty("Content-Type", contentType)
                    connection.outputStream.use { output ->
                        output.write(body)
                    }
                }

                val statusCode = connection.responseCode
                val stream =
                    if (statusCode in HTTP_SUCCESS_MIN..HTTP_SUCCESS_MAX) {
                        connection.inputStream
                    } else {
                        connection.errorStream ?: connection.inputStream
                    }

                val responseBody = stream?.use { input -> input.readBytes() } ?: ByteArray(0)

                if (statusCode !in HTTP_SUCCESS_MIN..HTTP_SUCCESS_MAX) {
                    val responseText = responseBody.toString(Charsets.UTF_8)
                    logDriveWarning(
                        "google drive request failed method=$method status=$statusCode url=$url",
                    )
                    throw DriveHttpException(statusCode, responseText)
                }

                DriveResponse(statusCode, responseBody)
            } finally {
                connection.disconnect()
            }
        }

    fun buildMultipartBody(
        boundary: String,
        metadata: JSONObject,
        data: ByteArray,
    ): ByteArray {
        val output = ByteArrayOutputStream()
        val prefix = "--$boundary\r\n"
        output.write(prefix.toByteArray())
        output.write("Content-Type: application/json; charset=UTF-8\r\n\r\n".toByteArray())
        output.write(metadata.toString().toByteArray())
        output.write("\r\n--$boundary\r\n".toByteArray())
        output.write("Content-Type: application/octet-stream\r\n\r\n".toByteArray())
        output.write(data)
        output.write("\r\n--$boundary--\r\n".toByteArray())
        return output.toByteArray()
    }

    suspend fun downloadFile(
        token: String,
        fileId: String,
    ): ByteArray =
        driveRequest(
            token = token,
            method = "GET",
            url = "${endpoints.filesEndpoint}/$fileId?alt=media",
        ).body

    private companion object {
        const val NETWORK_TIMEOUT_MS = 30_000
        const val HTTP_SUCCESS_MIN = 200
        const val HTTP_SUCCESS_MAX = 299
    }
}

internal fun DriveResponse.asJsonObject(): JSONObject =
    if (body.isEmpty()) {
        JSONObject()
    } else {
        JSONObject(body.toString(Charsets.UTF_8))
    }

internal data class DriveResponse(
    val statusCode: Int,
    val body: ByteArray,
)
