//
//  TapSigner.swift
//  Cove
//
//  Created by Praveen Perera on 3/13/25.
//

import CoreNFC
import Foundation

private let logger = Log(id: "TapCardNFC")

class TapSignerNFC {
    private var nfc: TapCardNFC

    init(tapcard: TapCard) {
        self.nfc = TapCardNFC(tapcard: tapcard)
    }

    public func setupTapSigner(_ factoryPin: String, _ newPin: String, _ chainCode: Data? = nil) async throws -> TapSignerResponse {
        var errorCount = 0
        var lastError: TapSignerReaderError? = nil

        // Create a continuation to bridge between async world and property changes
        let response = try await withCheckedThrowingContinuation { continuation in
            // Set up observation tracking before starting the operation
            Task {
                withObservationTracking {
                    // Access the properties to track them
                    _ = nfc.tapSignerResponse
                    _ = nfc.tapSignerError
                } onChange: {
                    // Re-register for changes
                    Task {
                        // Check if we got a response or error
                        if let response = self.nfc.tapSignerResponse {
                            continuation.resume(returning: response)
                            return
                        }

                        if let error = self.nfc.tapSignerError {
                            continuation.resume(throwing: error)
                            return
                        }
                    }
                }

                // Start the NFC operation
                do {
                    let cmd = try SetupCmd.tryNew(factoryPin: factoryPin, newPin: newPin, chainCode: chainCode)
                    nfc.tapSignerCmd = TapSignerCmd.setup(cmd)
                    nfc.scan()
                } catch let error as TapSignerReaderError {
                    throw error
                } catch {
                    throw TapSignerReaderError.Unknown(error.localizedDescription)
                }
            }
        }

        switch response {
        case .setup(.complete):
            nfc.session?.invalidate()
            return response
        case .setup(let incomplete):
            while true {
                var incompleteResponse = incomplete

                // convert this to a result type
                let response = await continueSetup(incompleteResponse)
                switch response {
                case .success(.setup(.complete(let c))):
                    nfc.session?.invalidate()
                    return .setup(.complete(c))

                case .success(.setup(let other)):
                    errorCount += 1
                    lastError = other.error
                    incompleteResponse = other

                case .failure(let error):
                    nfc.session?.invalidate()
                    Log.error("Error count: \(errorCount), last error: \(error)")
                    return .setup(incompleteResponse)
                }

                if errorCount > 5 {
                    nfc.session?.invalidate()
                    Log.error("Error count: \(errorCount), last error: \(lastError ?? .Unknown("unknown error, no error found"))")
                    return .setup(incompleteResponse)
                }
            }
        }
    }

    public func continueSetup(_ response: SetupCmdResponse) async -> Result<TapSignerResponse, TapSignerReaderError> {
        let cmd: SetupCmd? = switch response {
        case .continueFromInit(let c):
            c.continueCmd
        case .continueFromBackup(let c):
            c.continueCmd
        case .continueFromDerive(let c):
            c.continueCmd
        case .complete:
            .none
        }

        guard let cmd else { return .success(.setup(response)) }

        // Create a continuation to bridge between async world and property changes
        do {
            return try await withCheckedThrowingContinuation { continuation in
                // Set up observation tracking before starting the operation
                Task {
                    withObservationTracking {
                        // Access the properties to track them
                        _ = nfc.tapSignerResponse
                        _ = nfc.tapSignerError
                    } onChange: {
                        // Re-register for changes
                        Task {
                            // Check if we got a response or error
                            if let response = self.nfc.tapSignerResponse {
                                continuation.resume(returning: Result.success(response))
                                return
                            }

                            if let error = self.nfc.tapSignerError {
                                continuation.resume(returning: Result.failure(error))
                                return
                            }
                        }
                    }

                    // Start the NFC operation
                    nfc.tapSignerCmd = TapSignerCmd.setup(cmd)
                    nfc.scan()
                }
            }
        } catch let error as TapSignerReaderError {
            return .failure(error)
        } catch {
            return .failure(.Unknown(error.localizedDescription))
        }
    }
}

@Observable
private class TapCardNFC: NSObject, NFCTagReaderSessionDelegate {
    // private
    private var tag: NFCISO7816Tag?
    private var transport: TapCardTransport?

    // public
    public var session: NFCTagReaderSession?
    public let tapcard: TapCard
    public var isScanning: Bool = false

    public var tapSignerReader: TapSignerReader? = nil
    public var tapSignerCmd: TapSignerCmd? = nil
    public var tapSignerResponse: TapSignerResponse? = nil
    public var tapSignerError: TapSignerReaderError? = nil

    // public var satsCardReader: SatsCardReader? = nil
    // public var satsCardCmd: SatsCardCmd? = nil

    // cmd
    init(tapcard: TapCard) {
        self.tapcard = tapcard

        self.tapSignerReader = nil
        self.tapSignerCmd = nil

        //  self.satsCardReader = nil
        //  self.satsCardReader = nil

        self.tag = nil
        self.session = nil
        self.transport = nil
    }

    func scan() {
        guard let tapSignerCmd else { return Log.error("cmd not set") }
        logger.info("started scanning \(tapSignerCmd)")

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
            case .iso7816(let iso7816Tag):
                Log.debug("found tag iso7816")

                let readingMessage = "Reading tag, please hold still"
                session.alertMessage = readingMessage

                self.tag = iso7816Tag
                Task {
                    await self.createReader(from: iso7816Tag, session: session)
                }
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

    @MainActor
    private func createReader(from tag: NFCISO7816Tag, session: NFCTagReaderSession) async {
        do {
            let transport = TapCardTransport(session: session, tag: tag)
            switch tapcard {
            case .satsCard:
                (
                    // TODO: Implement SatsCardReader
                )

            case .tapSigner:
                let tapSignerReader = try await TapSignerReader(transport: transport, cmd: tapSignerCmd)
                self.tapSignerReader = tapSignerReader

                let response = try await tapSignerReader.run()
                tapSignerResponse = response
            }
        } catch let error as TapSignerReaderError {
            logger.error("TapSigner error: \(error)")
            tapSignerError = error
            session.invalidate(errorMessage: "TapSigner error: \(error.localizedDescription)")
        } catch {
            logger.error("Error creating reader: \(error)")
            session.invalidate(errorMessage: "Error creating reader: \(error.localizedDescription)")
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
        case .some(let error):
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

    func transmitApdu(commandApdu: Data) async throws -> Data {
        logger.debug("Transmitting APDU: \(commandApdu.count)")

        guard let apdu = NFCISO7816APDU(data: commandApdu) else {
            logger.error("Invalid APDU")
            return Data()
        }

        return try await withCheckedThrowingContinuation { continuation in
            tag.sendCommand(apdu: apdu) { response, sw1Value, sw2Value, error in
                if let error {
                    logger.error("APDU error: \(error)")
                    continuation.resume(throwing: error)
                    return
                }

                // Check for success (0x9000)
                let statusWord = (Int(sw1Value) << 8) | Int(sw2Value)
                if statusWord != 0x9000 {
                    // Handle specific error codes
                    var errorMessage = ""
                    switch statusWord {
                    case 0x6d00:
                        errorMessage = "Instruction code not supported or invalid"
                    default:
                        errorMessage =
                            if !response.isEmpty {
                                "Card error: SW=\(String(format: "0x%04X", statusWord)), data: \(response.hexEncodedString())"
                            } else {
                                "Card error: SW=\(String(format: "0x%04X", statusWord))"
                            }
                    }
                    logger.error(errorMessage)
                    continuation.resume(throwing: TransportError.CkTap(error: errorMessage, code: UInt64(statusWord)))
                    return
                }

                var fullResponse = response
                fullResponse.append(sw1Value)
                fullResponse.append(sw2Value)

                continuation.resume(returning: fullResponse)
            }
        }
    }
}
