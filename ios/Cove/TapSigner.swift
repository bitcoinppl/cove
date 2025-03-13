//
//  TapSigner.swift
//  Cove
//
//  Created by Praveen Perera on 3/13/25.
//

import CoreNFC
import Foundation

private let logger = Log(id: "TapCardNFC")

@Observable
class TapCardNFC: NSObject, NFCTagReaderSessionDelegate {
    private var tag: NFCISO7816Tag?
    private var session: NFCTagReaderSession?
    private var transport: TapCardTransport?

    // public
    public let tapcard: TapCard
    public var isScanning: Bool = false
    public var reader: TapCardReader? = nil

    init(tapcard: TapCard) {
        self.tapcard = tapcard

        self.reader = nil
        self.tag = nil
        self.session = nil
        self.transport = nil
    }

    func scan() {
        logger.info("started scanning")

        session = NFCTagReaderSession(pollingOption: [.iso14443, .iso15693], delegate: self)
        session?.alertMessage = "Hold your iPhone near the NFC tag."
        session?.begin()
    }

    func tagReaderSession(_ session: NFCTagReaderSession, didDetect tags: [NFCTag]) {
        self.session = session
        guard let tag = tags.first else {
            session.invalidate(errorMessage: "No tag detected.")
            return
        }

        session.connect(to: tag) { error in
            if let error {
                session.invalidate(
                    errorMessage:
                    "Connection error: \(error.localizedDescription), please try again")
                return
            }

            switch tag {
            case .iso15693:
                logger.error("found tag iso15693Tag")
                session.invalidate(errorMessage: "Unsupported tag type.")
            case let .iso7816(iso7816Tag):
                Log.debug("found tag iso7816")
                self.tag = iso7816Tag
                self.createReader(from: iso7816Tag, session: session)
            case .miFare:
                logger.error("found tag miFare")
                session.invalidate(errorMessage: "Unsupported tag type.")
            case .feliCa:
                logger.error("found tag feliCa")
                session.invalidate(errorMessage: "Unsupported tag type.")
            @unknown default:
                logger.error("unsupported tag type: \(tag)")
                session.invalidate(errorMessage: "Unsupported tag type.")
            }
        }
    }

    private func createReader(from tag: NFCISO7816Tag, session: NFCTagReaderSession) {
        do {
            let reader = try TapCardReader(transport: TapCardTransport(session: session, tag: tag))
            self.reader = reader
        } catch {
            logger.error("Error creating reader: \(error)")
        }
    }

    func tagReaderSessionDidBecomeActive(_: NFCTagReaderSession) {
        logger.debug("tapcard reader session did become active.")
    }

    func tagReaderSession(_ session: NFCTagReaderSession, didInvalidateWithError error: any Error) {
        Log.error("tapcard reader session did invalidate with error: \(error.localizedDescription)")
        switch error as? NFCReaderError {
        case .none:
            session.invalidate(errorMessage: "Unable to read NFC tag, try again")
        case let .some(error):
            switch error.code {
            case .readerTransceiveErrorTagConnectionLost:
                session.invalidate(
                    errorMessage: "Tag connection lost, please hold your phone still")
            default:
                session.invalidate(errorMessage: "Unable to read NFC tag, try again")
            }
        }
    }
}

/// Implements the TapcardTransportProtocol using the NFCTagReaderSession
class TapCardTransport: TapcardTransportProtocol, @unchecked Sendable {
    let nfcSession: NFCTagReaderSession
    var tag: NFCISO7816Tag

    init(session: NFCTagReaderSession, tag: NFCISO7816Tag) {
        self.nfcSession = session
        self.tag = tag
    }

    func transmitApdu(commandApdu: Data) -> Data {
        // Create the APDU command
        guard let apdu = NFCISO7816APDU(data: Data(commandApdu)) else {
            // Handle invalid APDU
            logger.error("Invalid APDU")
            return Data()
        }

        // We need to use a semaphore to make this synchronous
        let semaphore = DispatchSemaphore(value: 0)
        var responseData = Data()
        var sw1: UInt8 = 0
        var sw2: UInt8 = 0
        var commandError: Error?

        // Send the command
        tag.sendCommand(apdu: apdu) { response, sw1Value, sw2Value, error in
            responseData = response
            sw1 = sw1Value
            sw2 = sw2Value
            commandError = error
            semaphore.signal()
        }

        // Wait for the response
        _ = semaphore.wait(timeout: .now() + 5.0) // 5 second timeout

        if let error = commandError {
            // Handle error
            logger.error("APDU error: \(error)")
            return Data()
        }

        // Combine response data with status words
        var fullResponse = responseData
        fullResponse.append(sw1)
        fullResponse.append(sw2)

        return fullResponse
    }
}
