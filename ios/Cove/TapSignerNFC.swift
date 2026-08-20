//
//  TapSignerNFC.swift
//  Cove
//
//  Created by Praveen Perera on 3/13/25.
//

import CoreNFC
import CoveCore
import Foundation

private let logger = Log(id: "TapCardNFC")

private typealias TapSignerOperationResult = Result<TapSignerResponse, TapSignerReaderError>

/// Owns the one result channel for one NFC operation
private final class TapSignerOperationToken: @unchecked Sendable {
    typealias Continuation = CheckedContinuation<TapSignerOperationResult, Never>

    let id = UUID()

    private let lock = NSLock()
    private var continuation: Continuation?
    private var pendingResult: TapSignerOperationResult?
    private var resolved = false
    private var commandStarted = false

    deinit {
        resolve(.failure(.Unknown("TapSigner operation cancelled")))
    }

    var isResolved: Bool {
        lock.lock()
        defer { lock.unlock() }

        return resolved
    }

    var didStartCommand: Bool {
        lock.lock()
        defer { lock.unlock() }

        return commandStarted
    }

    func markCommandStarted() {
        lock.lock()
        commandStarted = true
        lock.unlock()
    }

    func install(_ continuation: Continuation) {
        lock.lock()
        let result = pendingResult
        pendingResult = nil

        if result == nil {
            self.continuation = continuation
        }
        lock.unlock()

        if let result {
            continuation.resume(returning: result)
        }
    }

    @discardableResult
    func resolve(_ result: TapSignerOperationResult) -> Bool {
        lock.lock()
        guard !resolved else {
            lock.unlock()
            return false
        }

        resolved = true
        let continuation = self.continuation
        self.continuation = nil

        if continuation == nil {
            pendingResult = result
        }
        lock.unlock()

        continuation?.resume(returning: result)
        return true
    }
}

@MainActor
class TapSignerNFC {
    private let nfc: TapCardNFC
    private var lastResponse_: TapSignerResponse?
    private var pendingRetry: TapSignerOperationContinuation?

    init(_ card: TapSigner) {
        Log.debug("Initializing TapSignerNFC")
        nfc = TapCardNFC(tapcard: .tapSigner(card))
    }

    var isScanning: Bool {
        nfc.isScanning
    }

    func setupTapSigner(factoryPin: String, newPin: String, chainCode: Data) async
        -> Result<SetupCmdResponse, TapSignerReaderError>
    {
        clearLastResponse()

        do {
            let factoryCvc = try makeTapSignerCvc(value: factoryPin)
            let newCvc = try makeTapSignerCvc(value: newPin)
            let setupCmd = try SetupCmd.tryNew(
                factoryCvc: factoryCvc,
                newCvc: newCvc,
                chainCode: chainCode
            )

            return await doSetupTapSigner(cmd: setupCmd)
        } catch let error as TapSignerReaderError {
            return .failure(error)
        } catch let error as TapSignerCvcInputError {
            return .failure(.Unknown(error.errorDescription ?? "Invalid TAPSIGNER CVC"))
        } catch {
            return .failure(.Unknown("TapSigner setup failed"))
        }
    }

    func derive(pin: String) async -> Result<DeriveInfo, TapSignerReaderError> {
        clearLastResponse()

        do {
            let cvc = try makeTapSignerCvc(value: pin)
            return await performTapSignerCmd(cmd: .derive(cvc: cvc)) {
                $0.deriveResponse
            }
        } catch let error as TapSignerCvcInputError {
            return .failure(.Unknown(error.errorDescription ?? "Invalid TAPSIGNER CVC"))
        } catch {
            return .failure(.Unknown("TapSigner import failed"))
        }
    }

    func changePin(currentPin: String, newPin: String) async -> Result<
        Void, TapSignerReaderError
    > {
        do {
            let currentCvc = try makeTapSignerCvc(value: currentPin)
            let newCvc = try makeTapSignerCvc(value: newPin)

            return await performTapSignerCmd(
                cmd: .change(currentCvc: currentCvc, newCvc: newCvc),
                continuationMatches: {
                    $0.matchesChange(currentCvc: currentCvc, newCvc: newCvc)
                }
            ) {
                $0.isChangeResponse ? true : nil
            }.map { _ in () }
        } catch let error as TapSignerCvcInputError {
            return .failure(.Unknown(error.errorDescription ?? "Invalid TAPSIGNER CVC"))
        } catch {
            return .failure(.Unknown("TapSigner PIN change failed"))
        }
    }

    func backup(pin: String) async -> Result<Data, TapSignerReaderError> {
        do {
            let cvc = try makeTapSignerCvc(value: pin)

            return await performTapSignerCmd(
                cmd: .backup(cvc: cvc),
                continuationMatches: { $0.matchesBackup(cvc: cvc) }
            ) {
                $0.backupResponse
            }
        } catch let error as TapSignerCvcInputError {
            return .failure(.Unknown(error.errorDescription ?? "Invalid TAPSIGNER CVC"))
        } catch {
            return .failure(.Unknown("TapSigner backup failed"))
        }
    }

    func sign(psbt: Psbt, pin: String) async -> Result<Psbt, TapSignerReaderError> {
        clearLastResponse()

        do {
            let cvc = try makeTapSignerCvc(value: pin)
            return await performTapSignerCmd(cmd: .sign(psbt: psbt, cvc: cvc)) {
                $0.signResponse
            }
        } catch let error as TapSignerCvcInputError {
            return .failure(.Unknown(error.errorDescription ?? "Invalid TAPSIGNER CVC"))
        } catch {
            return .failure(.Unknown("TapSigner signing failed"))
        }
    }

    func cancel() {
        clearLastResponse()
        nfc.cancelActiveOperation()
    }

    private func clearLastResponse() {
        pendingRetry = nil
        lastResponse_ = nil
        nfc.clearLastResponse()
    }

    func lastResponse() -> TapSignerResponse? {
        lastResponse_ ?? nfc.lastResponse
    }

    private func performTapSignerCmd<T>(
        cmd: TapSignerCmd,
        continuationMatches: ((TapSignerOperationContinuation) -> Bool)? = nil,
        _ successResult: @escaping (TapSignerResponse) -> T?
    ) async -> Result<T, TapSignerReaderError> {
        var command: TapSignerCmd
        if let continuationMatches,
           let pendingRetry,
           pendingRetry.canRetry(),
           continuationMatches(pendingRetry)
        {
            command = .continueOperation(pendingRetry)
        } else {
            clearLastResponse()
            command = cmd
        }

        var retryCount = 0

        while true {
            switch await runTapSignerCmd(command) {
            case let .failure(error):
                if pendingRetry?.canRetry() == false {
                    pendingRetry = nil
                }

                return .failure(error)
            case let .success(response):
                if let result = successResult(response) {
                    pendingRetry = nil
                    return .success(result)
                }

                guard case let .retry(continuation) = response else {
                    Log.error("TapSigner operation returned an unexpected response")
                    pendingRetry = nil
                    return .failure(.Unknown("TapSigner operation returned no result"))
                }

                guard retryCount < 5 else {
                    Log.error("TapSigner operation retry limit reached")
                    pendingRetry = nil
                    return .failure(.ManualRecoveryRequired)
                }

                retryCount += 1
                command = .continueOperation(continuation)
                if continuationMatches != nil {
                    pendingRetry = continuation
                }
            }
        }
    }

    private func doSetupTapSigner(cmd: SetupCmd) async -> Result<
        SetupCmdResponse, TapSignerReaderError
    > {
        var response = await runTapSignerCmd(.setup(cmd))
        var retryCount = 0

        while true {
            switch response {
            case let .failure(error):
                return .failure(error)
            case let .success(.setup(setupResponse)):
                lastResponse_ = .setup(setupResponse)

                switch setupResponse {
                case .complete:
                    return .success(setupResponse)
                case let .retry(continuation):
                    guard retryCount < 5 else {
                        Log.error("TapSigner setup retry limit reached")
                        return .success(setupResponse)
                    }

                    retryCount += 1
                    response = await runTapSignerCmd(.continueSetup(continuation))
                }
            case .success:
                Log.error("TapSigner setup returned an unexpected response")
                return .failure(.Unknown("TapSigner setup returned no result"))
            }
        }
    }

    func continueSetup(_ response: SetupCmdResponse) async -> Result<
        SetupCmdResponse, TapSignerReaderError
    > {
        guard case let .retry(continuation) = response else { return .success(response) }

        switch await runTapSignerCmd(.continueSetup(continuation)) {
        case let .failure(error):
            return .failure(error)
        case let .success(.setup(nextResponse)):
            return .success(nextResponse)
        case .success:
            return .failure(.Unknown("TapSigner setup returned no result"))
        }
    }

    private func runTapSignerCmd(_ command: TapSignerCmd) async -> TapSignerOperationResult {
        let operation = TapSignerOperationToken()

        let result = await withTaskCancellationHandler(operation: {
            await withCheckedContinuation { (continuation: TapSignerOperationToken.Continuation) in
                operation.install(continuation)
                Task { @MainActor in
                    nfc.start(command: command, operation: operation)
                }
            }
        }, onCancel: {
            let error = TapSignerReaderError.Unknown("TapSigner operation cancelled")
            operation.resolve(.failure(error))
            Task { @MainActor in
                nfc.cancel(operation)
            }
        })

        if case let .success(response) = result {
            lastResponse_ = response
        }

        return result
    }
}

@MainActor
@Observable
private class TapCardNFC: NSObject, NFCTagReaderSessionDelegate {
    private var tag: NFCISO7816Tag?
    private var transport: TapCardTransport?
    private var activeOperation: TapSignerOperationToken?
    private var command: TapSignerCmd?
    private var tapSignerReader: TapSignerReader?
    private var session: NFCTagReaderSession?

    let tapcard: TapCard
    var isScanning = false
    var lastResponse: TapSignerResponse?

    init(tapcard: TapCard) {
        self.tapcard = tapcard
    }

    func start(command: TapSignerCmd, operation: TapSignerOperationToken) {
        guard !operation.isResolved else { return }

        guard activeOperation == nil else {
            operation.resolve(.failure(.OperationInProgress))
            return
        }

        clearState()
        activeOperation = operation
        self.command = command

        logger.info("started scanning for \(operationKind(command))")
        isScanning = true
        session = NFCTagReaderSession(pollingOption: [.iso14443, .iso15693], delegate: self)
        session?.alertMessage = scanMessage(for: command)
        session?.begin()
    }

    func cancel() {
        cancelActiveOperation()
    }

    func cancel(_ operation: TapSignerOperationToken) {
        guard activeOperation === operation else { return }

        finish(
            operation,
            result: .failure(.Unknown("TapSigner operation cancelled")),
            invalidateSession: true
        )
    }

    func cancelActiveOperation() {
        guard let operation = activeOperation else {
            clearState()
            return
        }

        finish(
            operation,
            result: .failure(.Unknown("TapSigner operation cancelled")),
            invalidateSession: true
        )
    }

    func clearLastResponse() {
        lastResponse = nil
    }

    private func clearState() {
        activeOperation = nil
        command = nil
        tapSignerReader = nil
        tag = nil
        transport = nil
        session = nil
        isScanning = false
        lastResponse = nil
    }

    private func finish(
        _ operation: TapSignerOperationToken,
        result: TapSignerOperationResult,
        invalidateSession: Bool,
        errorMessage: String? = nil,
        session: NFCTagReaderSession? = nil
    ) {
        guard activeOperation === operation else { return }

        let activeSession = session ?? self.session
        var response: TapSignerResponse?
        if case let .success(value) = result {
            response = value
        }

        clearState()
        lastResponse = response
        operation.resolve(result)

        guard invalidateSession else { return }
        if let errorMessage {
            activeSession?.invalidate(errorMessage: errorMessage)
        } else {
            activeSession?.invalidate()
        }
    }

    private func operationKind(_ command: TapSignerCmd) -> String {
        switch command {
        case .setup: "setup"
        case .continueSetup: "continue_setup"
        case .continueOperation: "continue_operation"
        case .derive: "derive"
        case .change: "change"
        case .backup: "backup"
        case .sign: "sign"
        }
    }

    private func scanMessage(for command: TapSignerCmd) -> String {
        switch command {
        case let .continueSetup(continuation): continuation.message()
        case let .continueOperation(continuation): continuation.message()
        default: "Hold your iPhone near the NFC tag."
        }
    }

    nonisolated func tagReaderSession(_ session: NFCTagReaderSession, didDetect tags: [NFCTag]) {
        let tag = tags.first

        Task { @MainActor [weak self, session, tag] in
            self?.handleDetectedTag(session: session, tag: tag)
        }
    }

    private func handleDetectedTag(session: NFCTagReaderSession, tag: NFCTag?) {
        guard self.session === session else {
            session.invalidate()
            return
        }

        guard let operation = activeOperation else {
            session.invalidate()
            return
        }

        guard let tag else {
            finish(
                operation,
                result: .failure(.Unknown("No tag detected")),
                invalidateSession: true,
                errorMessage: "No tag detected."
            )
            return
        }

        session.connect(to: tag) { error in
            let didConnect = error == nil

            Task { @MainActor [weak self, session, tag, operation, didConnect] in
                self?.handleConnectionResult(
                    didConnect: didConnect,
                    tag: tag,
                    session: session,
                    operation: operation
                )
            }
        }
    }

    private func handleConnectionResult(
        didConnect: Bool,
        tag: NFCTag,
        session: NFCTagReaderSession,
        operation: TapSignerOperationToken
    ) {
        guard self.session === session, activeOperation === operation else { return }

        guard didConnect else {
            finish(
                operation,
                result: .failure(.Unknown("Unable to connect to TapSigner")),
                invalidateSession: true,
                errorMessage: "Unable to connect to TapSigner. Please try again.",
                session: session
            )
            return
        }

        handleConnectedTag(tag, session: session, operation: operation)
    }

    private func handleConnectedTag(
        _ tag: NFCTag,
        session: NFCTagReaderSession,
        operation: TapSignerOperationToken
    ) {
        switch tag {
        case .iso15693:
            rejectUnsupportedTag("found tag iso15693Tag", session: session, operation: operation)
        case let .iso7816(iso7816Tag):
            Log.debug("found tag iso7816")
            session.alertMessage = "Reading tag, please hold still"
            self.tag = iso7816Tag

            Task { @MainActor [weak self, iso7816Tag, session, operation] in
                await self?.createReader(
                    from: iso7816Tag,
                    session: session,
                    operation: operation
                )
            }
        case .miFare:
            rejectUnsupportedTag("found tag miFare", session: session, operation: operation)
        case .feliCa:
            rejectUnsupportedTag("found tag feliCa", session: session, operation: operation)
        @unknown default:
            rejectUnsupportedTag("unsupported tag type", session: session, operation: operation)
        }
    }

    private func rejectUnsupportedTag(
        _ logMessage: String,
        session: NFCTagReaderSession,
        operation: TapSignerOperationToken
    ) {
        logger.error("\(logMessage)")
        finish(
            operation,
            result: .failure(.Unknown("Unsupported tag type")),
            invalidateSession: true,
            errorMessage: "Unsupported tag type.",
            session: session
        )
    }

    @MainActor
    private func createReader(
        from tag: NFCISO7816Tag,
        session: NFCTagReaderSession,
        operation: TapSignerOperationToken
    ) async {
        guard activeOperation === operation, let command else { return }

        do {
            let transport = TapCardTransport(session: session, tag: tag)
            self.transport = transport

            switch tapcard {
            case .satsCard:
                // satscard is not part of this flow yet
                finish(
                    operation,
                    result: .failure(.Unknown("Unsupported card type")),
                    invalidateSession: true,
                    errorMessage: "Unsupported card type.",
                    session: session
                )
                return
            case .tapSigner:
                let reader = try await createTapSignerReader(
                    transport: transport,
                    cmd: command
                )

                guard activeOperation === operation else { return }
                tapSignerReader = reader
                operation.markCommandStarted()
                let response = try await reader.run()
                finish(operation, result: .success(response), invalidateSession: true)
            }
        } catch let error as TapSignerReaderError {
            let message = error.isAuthError()
                ? "Wrong PIN, please try again"
                : "TapSigner operation failed. Please try again."
            logger.error("TapSigner operation failed: \(operationKind(command))")
            finish(
                operation,
                result: .failure(error),
                invalidateSession: true,
                errorMessage: message,
                session: session
            )
        } catch is CancellationError {
            finish(
                operation,
                result: .failure(.Unknown("TapSigner operation cancelled")),
                invalidateSession: true,
                session: session
            )
        } catch {
            logger.error("TapSigner operation failed while creating reader")
            finish(
                operation,
                result: .failure(.Unknown("Unable to read TapSigner")),
                invalidateSession: true,
                errorMessage: "Unable to read TapSigner, please try again",
                session: session
            )
        }
    }

    nonisolated func tagReaderSessionDidBecomeActive(_: NFCTagReaderSession) {
        logger.debug("tapcard reader session did become active")
    }

    nonisolated func tagReaderSession(
        _ session: NFCTagReaderSession,
        didInvalidateWithError error: any Error
    ) {
        let result: TapSignerOperationResult = if let nfcError = error as? NFCReaderError {
            switch nfcError.code {
            case .readerSessionInvalidationErrorUserCanceled:
                .failure(.Unknown("TapSigner operation cancelled"))
            case .readerSessionInvalidationErrorSessionTimeout:
                .failure(.Unknown("TapSigner scan timed out"))
            case .readerTransceiveErrorTagConnectionLost:
                .failure(.Unknown("Tag connection lost, please hold your phone still"))
            default:
                .failure(.Unknown("Unable to read NFC tag, try again"))
            }
        } else {
            .failure(.Unknown("Unable to read NFC tag, try again"))
        }

        Task { @MainActor [weak self, session, result] in
            self?.handleInvalidation(session: session, result: result)
        }
    }

    private func handleInvalidation(
        session: NFCTagReaderSession,
        result: TapSignerOperationResult
    ) {
        guard self.session === session, let operation = activeOperation else { return }

        if operation.didStartCommand {
            logger.debug("tapcard reader session invalidated during an active command")
            return
        }

        if case .failure(.Unknown("TapSigner operation cancelled")) = result {
            logger.debug("tapcard reader session ended normally")
        }

        finish(operation, result: result, invalidateSession: false, session: session)
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
        nfcSession.alertMessage += message
    }

    func transmitApdu(commandApdu: Data) async throws -> Data {
        logger.debug("Transmitting APDU, bytes=\(commandApdu.count)")

        guard let apdu = NFCISO7816APDU(data: commandApdu) else {
            logger.error("Invalid APDU")
            throw TransportError.UnknownError("Unable to communicate with TapSigner")
        }

        return try await withCheckedThrowingContinuation { continuation in
            tag.sendCommand(apdu: apdu) { response, sw1Value, sw2Value, error in
                Log.debug("APDU response: \(response.count) bytes")

                if error != nil {
                    logger.error("TapSigner APDU transmission failed")
                    continuation.resume(
                        throwing: TransportError.UnknownError(
                            "Unable to communicate with TapSigner, please try again"
                        )
                    )
                    return
                }

                let statusWord = (Int(sw1Value) << 8) | Int(sw2Value)
                if statusWord != 0x9000 {
                    logger.error("TapSigner APDU operation rejected")
                    continuation.resume(
                        throwing: TransportError.UnknownStatusWord(
                            code: UInt16(statusWord),
                            detail: "TapSigner card rejected the operation"
                        )
                    )
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
