//
//  TapSignerNfc.swift
//  Cove
//
//  Created by Praveen Perera on 3/13/25.
//

import CoreNFC
import Foundation

private let logger = Log(id: "TapCardNFC")

class TapSignerNFC {
    private var nfc: TapCardNFC
    private var lastResponse_: TapSignerResponse?

    init(_ card: TapSigner) {
        nfc = TapCardNFC(tapcard: .tapSigner(card))
    }

    public func setupTapSigner(factoryPin: String, newPin: String, chainCode: Data? = nil) async
        -> Result<SetupCmdResponse, TapSignerReaderError>
    {
        do {
            return try await .success(
                doSetupTapSigner(factoryPin: factoryPin, newPin: newPin, chainCode: chainCode))
        } catch let error as TapSignerReaderError {
            return .failure(error)
        } catch {
            return .failure(TapSignerReaderError.Unknown(error.localizedDescription))
        }
    }

    public func derive(pin: String) async -> Result<DeriveInfo, TapSignerReaderError> {
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
                            if let response = self.nfc.tapSignerResponse?.deriveResponse {
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
                    nfc.tapSignerCmd = TapSignerCmd.derive(pin: pin)
                    nfc.scan()
                }
            }
        } catch let error as TapSignerReaderError {
            return Result.failure(error)
        } catch {
            return Result.failure(TapSignerReaderError.Unknown(error.localizedDescription))
        }
    }

    public func lastResponse() -> TapSignerResponse? {
        nfc.tapSignerReader?.lastResponse() ?? lastResponse_
    }

    private func doSetupTapSigner(factoryPin: String, newPin: String, chainCode: Data? = nil)
        async throws -> SetupCmdResponse
    {
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
                        if let response = self.nfc.tapSignerResponse?.setupResponse {
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
                    let cmd = try SetupCmd.tryNew(
                        factoryPin: factoryPin, newPin: newPin, chainCode: chainCode)
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
        case .complete:
            nfc.session?.invalidate()
            return response
        case let incomplete:
            while true {
                var incompleteResponse = incomplete
                lastResponse_ = .setup(incompleteResponse)

                // convert this to a result type
                let response = await continueSetup(incompleteResponse)
                switch response {
                case let .success(.complete(c)):
                    nfc.session?.invalidate()
                    return .complete(c)

                case let .success(other):
                    errorCount += 1
                    lastError = other.error
                    incompleteResponse = other

                case let .failure(error):
                    nfc.session?.invalidate()
                    Log.error("Error count: \(errorCount), last error: \(error)")
                    return incompleteResponse
                }

                if errorCount > 5 {
                    nfc.session?.invalidate()
                    Log.error(
                        "Error count: \(errorCount), last error: \(lastError ?? .Unknown("unknown error, no error found"))"
                    )
                    return incompleteResponse
                }
            }
        }
    }

    public func continueSetup(_ response: SetupCmdResponse) async -> Result<
        SetupCmdResponse, TapSignerReaderError
    > {
        let cmd: SetupCmd? =
            switch response {
            case let .continueFromInit(c):
                c.continueCmd
            case let .continueFromBackup(c):
                c.continueCmd
            case let .continueFromDerive(c):
                c.continueCmd
            case .complete:
                .none
            }

        guard let cmd else { return .success(response) }

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
                            if let response = self.nfc.tapSignerResponse?.setupResponse {
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

        tapSignerReader = nil
        tapSignerCmd = nil

        //  self.satsCardReader = nil
        //  self.satsCardReader = nil

        tag = nil
        session = nil
        transport = nil
    }

    func scan() {
        guard let tapSignerCmd else { return Log.error("cmd not set") }
        switch tapSignerCmd {
        case .setup: logger.info("started scanning for setup")
        case .derive: logger.info("started scanning for derive")
        }

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
                ( // TODO: Implement SatsCardReader
                )

            case .tapSigner:
                let tapSignerReader = try await TapSignerReader(
                    transport: transport, cmd: tapSignerCmd)

                self.tapSignerReader = tapSignerReader

                let response = try await tapSignerReader.run()
                tapSignerResponse = response
            }
        } catch let error as TapSignerReaderError {
            logger.error("TAPSIGNER error: \(error)")
            tapSignerError = error
            if case .TapSignerError(.CkTap(.BadAuth)) = error {
                return session.invalidate(errorMessage: "Wrong PIN, please try again")
            }
            session.invalidate(errorMessage: "TapSigner error: \(error.describe)")
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
            tapSignerError = .Unknown("Unable to read NFC tag, try again")
            session.invalidate(errorMessage: "Unable to read NFC tag, try again")
        case let .some(error):
            switch error.code {
            case .readerTransceiveErrorTagConnectionLost:
                tapSignerError = .Unknown("Tag connection lost, please hold your phone still")
                session.invalidate(
                    errorMessage: "Tag connection lost, please hold your phone still")
            default:
                tapSignerError = .Unknown("Unable to read NFC tag, try again")
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
        nfcSession = session
        self.tag = tag
    }

    func setMessage(message: String) {
        nfcSession.alertMessage = message
    }

    func appendMessage(message: String) {
        let msg = nfcSession.alertMessage
        nfcSession.alertMessage = msg + message
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
                    case 0x6D00:
                        errorMessage = "Instruction code not supported or invalid"
                    default:
                        errorMessage =
                            if !response.isEmpty {
                                "Card error: SW=\(String(format: "0x%04X", statusWord)), data: \(response.hexEncodedString())"
                            } else {
                                "Card error: SW=\(String(format: "0x%04X", statusWord))"
                            }
                    }

                    logger.error("APDU ERROR: \(errorMessage)")
                    continuation.resume(
                        throwing: TransportError(code: statusWord, message: errorMessage))
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
