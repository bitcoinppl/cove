import CoreNFC
import SwiftUI

class NFCWriter: NSObject, NFCNDEFReaderSessionDelegate, ObservableObject {
    var data: Data?
    var session: NFCNDEFReaderSession?

    var isScanning: Bool = false

    private let logger = Log(id: "NfcWriter")
    private var task: Task<Void, Never>?

    deinit {
        isScanning = false
    }

    func writeToTag(data: Data) {
        guard !isScanning else { return }

        isScanning = true

        Log.debug("Writing to NFC tag, with data of size: \(data.count)")
        self.data = data
        guard NFCNDEFReaderSession.readingAvailable else {
            isScanning = false
            return
        }

        session = NFCNDEFReaderSession(delegate: self, queue: nil, invalidateAfterFirstRead: false)
        session?.alertMessage = "Hold your iPhone near an NFC tag to write"
        session?.begin()
    }

    func readerSession(_: NFCNDEFReaderSession, didDetectNDEFs _: [NFCNDEFMessage]) {
        logger.debug("detected NDEFs, ignoring")
    }

    func readerSession(_ session: NFCNDEFReaderSession, didDetect tags: [NFCNDEFTag]) {
        logger.debug("detected \(tags.count) tags, continuing...")

        guard let data, !data.isEmpty else {
            session.invalidate(errorMessage: "No data to write to NFC tag")
            return
        }

        guard let tag = tags.first else {
            session.invalidate(errorMessage: "No tag found")
            return
        }

        session.connect(to: tag) { error in
            self.logger.debug("connected to tag")
            let message = "Writing to tag, please hold still..."
            session.alertMessage = message

            if let error {
                session.invalidate(errorMessage: error.localizedDescription)
                return
            }

            tag.queryNDEFStatus { _, _, error in
                guard error == nil else {
                    session.invalidate(errorMessage: "Failed to query tag")
                    return
                }

                // Use a single payload with chunkSize parameter
                let payload = NFCNDEFPayload(
                    format: .media,
                    type: "application/octet-stream".data(using: .utf8)!,
                    identifier: Data(),
                    payload: data
                )

                let message = NFCNDEFMessage(records: [payload])
                self.logger.debug("Writing message with \(message.records.count) records")

                tag.writeNDEF(message) { error in
                    if let error {
                        self.isScanning = false
                        session.invalidate(errorMessage: "Write failed: \(error.localizedDescription)")
                    } else {
                        self.isScanning = false
                        session.alertMessage = "Successfully wrote to tag!"
                        session.invalidate()
                        self.task?.cancel()
                    }
                }
            }
        }
    }

    func readerSession(_ session: NFCNDEFReaderSession, didInvalidateWithError _: Error) {
        isScanning = false
        logger.debug("Session did invalidate with error")
        session.invalidate()
    }

    func tagReaderSession(_: NFCTagReaderSession, didInvalidateWithError error: any Error) {
        isScanning = false
        logger.error("Tag reader session did invalidate with error: \(error.localizedDescription)")
        task?.cancel()
    }
}
