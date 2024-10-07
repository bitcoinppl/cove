//
//  NFCReader.swift
//  Cove
//
//  Created by Praveen Perera on 9/26/24.
//

import CoreNFC
import Foundation

@Observable
class NFCReader: NSObject, NFCTagReaderSessionDelegate {
    let consts: NfcConst
    let blocksToRead: Int

    var reader: FfiNfcReader
    var session: NFCTagReaderSession?

    var scannedMessage: String?
    var retries = 0
    var readBytes: Data
    var messageInfo: MessageInfo?

    var readingMessage = ""
    var currentBlock = 0

    // should continue reading next block
    enum ParsingState {
        case incomplete
        case complete
        case error
    }

    override init() {
        Log.debug("create nfc reader")
        consts = NfcConst()
        blocksToRead = Int(consts.numberOfBlocksPerChunk())
        reader = FfiNfcReader()
        readBytes = Data()
    }

    func scan() {
        scannedMessage = nil
        session = NFCTagReaderSession(pollingOption: [.iso14443, .iso15693], delegate: self)
        session?.alertMessage = "Hold your iPhone near the NFC tag."
        session?.begin()
    }

    func resetReader() {
        Log.debug("reset reader")
        scannedMessage = nil
        reader = FfiNfcReader()
        readBytes = Data()
        currentBlock = 0
    }

    func tagReaderSession(_ session: NFCTagReaderSession, didDetect tags: [NFCTag]) {
        guard let tag = tags.first else {
            session.invalidate(errorMessage: "No tag detected.")
            return
        }

        session.connect(to: tag) { error in
            if let error = error {
                session.invalidate(errorMessage: "Connection error: \(error.localizedDescription), please try again")
                return
            }

            switch tag {
            case let .iso15693(iso15693Tag):
                Log.debug("found tag iso15693: \(iso15693Tag)")
                self.readBlocks(tag: iso15693Tag, session: session)
            case let .iso7816(iso7816Tag):
                Log.debug("found tag iso7816")
                self.readNDEF(from: iso7816Tag, session: session)
            case let .miFare(miFareTag):
                Log.debug("found tag miFare")
                self.readNDEF(from: miFareTag, session: session)
            case let .feliCa(feliCaTag):
                Log.debug("found tag feliCa")
                self.readNDEF(from: feliCaTag, session: session)
            @unknown default:
                Log.error("unsupported tag type: \(tag)")
                session.invalidate(errorMessage: "Unsupported tag type.")
            }
        }
    }

    func readBlocks(tag: NFCISO15693Tag, session: NFCTagReaderSession) {
        readingMessage = "Reading tag, please hold still"
        session.alertMessage = readingMessage

        // when readBlocks is called if the old one is in started status then this might be the user trying to scan the same tag again
        Log.debug("\(reader.isStarted()), b: \(readBytes.count)")
        if reader.isStarted() && !readBytes.isEmpty {
            // read the first block chunk
            tag.readMultipleBlocks(requestFlags: .highDataRate, blockRange: NSRange(location: 0, length: blocksToRead)) { data, error in
                // error try again
                if error != nil {
                    self.retries += 1
                    if self.retries < 10 {
                        Log.error("Error reading block: \(error!.localizedDescription)")
                        self.readBlocks(tag: tag, session: session)
                    }

                    return
                }

                // succesful read, reset retries
                self.retries = 0

                // is resumable set the currentBlock to how much data we already have
                let data = data.flatMap { $0 }
                if (try? self.reader.isResumeable(data: Data(data))) != nil {
                    Log.info("Resuming from block: \(self.currentBlock)")
                } else {
                    // reset reader and bytes read
                    Log.warn("Trying to scan a different NFC message, resetting")
                    self.resetReader()
                }
            }
        }

        func readNextBlock() {
            Log.debug("current block: \(currentBlock)")
            // already complete
            if scannedMessage != nil {
                Log.debug("scanning complete")
                return
            }

            let blockRange = NSRange(location: currentBlock, length: blocksToRead)

            tag.extendedReadMultipleBlocks(requestFlags: .highDataRate, blockRange: blockRange) { data, error in
                // if there is an error, add it to the result
                let result: Result<[Data], any Error> = {
                    if let error = error {
                        return .failure(error)
                    }

                    return .success(data)
                }()

                switch result {
                // succesfully read the raw bytes, lets handle the bytes
                case let .success(data):
                    self.readingMessage = self.readingMessage.appending(".")
                    session.alertMessage = self.readingMessage

                    let dataChunk = data.flatMap { $0 }
                    self.currentBlock = self.currentBlock + self.blocksToRead

                    self.retries = 0
                    self.readBytes.append(contentsOf: dataChunk)

                    // has read enough data to get the message
                    if let messageInfo = self.messageInfo, self.readBytes.count >= messageInfo.totalPayloadLength {
                        if case .error = self.parseAndHandleResult(session: session) {
                            return
                        }
                    }

                    if self.messageInfo == nil {
                        if case .error = self.parseAndHandleResult(session: session) {
                            Log.warn("Trying to read TAG in unsupported format, falling back to built in NDEF reader")
                            return self.readNDEF(from: tag, session: session)
                        }
                    }

                    readNextBlock()

                // problem physically reading the data, so lets retry
                case let .failure(error):
                    if self.retries < 10 {
                        Log.warn("read error: \(error.localizedDescription), retrying")
                        self.retries = self.retries + 1
                        readNextBlock()
                    } else {
                        Log.error("read error, retries exhausted: \(error.localizedDescription)")
                        self.tagReaderSession(session, didInvalidateWithError: error)
                    }
                }
            }
        } // END: ReadNextBlock

        // start calling the readNextBlock() recursive function
        readNextBlock()
    }

    func parseAndHandleResult(session: NFCTagReaderSession) -> ParsingState {
        switch Result(catching: { try self.reader.parse(data: self.readBytes) }) {
        case let .success(.incomplete(result)):
            messageInfo = result.messageInfo
            readBytes = result.leftOverBytes
            return .incomplete
        case let .success(.complete(_, records)):
            resetReader()
            scannedMessage = reader.stringFromRecord(record: records.first!)
            session.invalidate()
            return .complete
        case let .failure(error):
            // error, lets result and invalidate
            resetReader()
            tagReaderSession(session, didInvalidateWithError: error)
            return .error
        }
    }

    // fallback function
    func readNDEF<T: NFCNDEFTag>(from tag: T, session: NFCTagReaderSession) {
        Log.debug("reading NDEF message from tag: \(tag)")
        session.alertMessage = "Reading data please hold still..."

        tag.readNDEF { message, error in
            if let error = error {
                if message == nil {
                    Log.error("read error: \(error.localizedDescription)")
                    session.invalidate(errorMessage: "Unable to read NFC tag please try again.")
                    return
                }
            }

            guard let message = message else {
                Log.error("no NDEF message found")
                session.invalidate(errorMessage: "Unable to read NFC tag please try again.")
                return
            }

            self.processNDEFMessage(message)
            if self.scannedMessage != nil {
                DispatchQueue.main.async {
                    session.alertMessage = "Tag read successfully!"
                    session.invalidate()
                }
            }
        }
    }

    func processNDEFMessage(_ message: NFCNDEFMessage) {
        Log.debug("processing NDEF message, \(message)")
        var _message = ""

        print("num of records: \(message.records.count)")

        for record in message.records {
            print("Record type: \(record.typeNameFormat)")
            if let type = String(data: record.type, encoding: .utf8) {
                _message += "\(type): "
                print("Type: \(type)")
            }
            if let payload = String(data: record.payload, encoding: .utf8) {
                _message += "\(payload)\n"
                print("Payload: \(payload)")
            }

            print("ID: \(record.identifier)")
            _message += "----\n"
            print("---")
        }

        scannedMessage = _message
    }

    func tagReaderSessionDidBecomeActive(_: NFCTagReaderSession) {
        Log.debug("Tag reader session did become active.")
    }

    func tagReaderSession(_ session: NFCTagReaderSession, didInvalidateWithError error: any Error) {
        Log.error("Tag reader session did invalidate with error: \(error.localizedDescription)")
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
