//
//  NfcReader.swift
//  Cove
//
//  Created by Praveen Perera on 9/26/24.
//

import CoreNFC
import Foundation

import CoreNFC

@Observable
class NFCReader: NSObject, NFCTagReaderSessionDelegate {
    var session: NFCTagReaderSession?
    var scannedMessage: String?
    var retries = 0
    
    var blocksToRead = 32
    
    func scan() {
        session = NFCTagReaderSession(pollingOption: [.iso14443, .iso15693], delegate: self)
        session?.alertMessage = "Hold your iPhone near the NFC tag."
        session?.begin()
    }
    
    func tagReaderSession(_ session: NFCTagReaderSession, didDetect tags: [NFCTag]) {
        guard let tag = tags.first else {
            session.invalidate(errorMessage: "No tag detected.")
            return
        }
        
        session.connect(to: tag) { error in
            if let error = error {
                session.invalidate(errorMessage: "Connection error: \(error.localizedDescription)")
                return
            }
            
            switch tag {
            case .iso15693(let iso15693Tag):
                Log.debug("found tag iso15693: \(iso15693Tag)")
                self.readBlocks(tag: iso15693Tag, session: session)
//                self.readNDEF(from: iso15693Tag, session: session)
            case .iso7816(let iso7816Tag):
                Log.debug("found tag iso7816")
                self.readNDEF(from: iso7816Tag, session: session)
            case .miFare(let miFareTag):
                Log.debug("found tag miFare")
                self.readNDEF(from: miFareTag, session: session)
            case .feliCa(let feliCaTag):
                Log.debug("found tag feliCa")
                self.readNDEF(from: feliCaTag, session: session)
            @unknown default:
                Log.error("unsupported tag type: \(tag)")
                session.invalidate(errorMessage: "Unsupported tag type.")
            }
        }
    }
    
    func readBlocks(tag: NFCISO15693Tag, session: NFCTagReaderSession) {
        session.alertMessage = "Reading tag, please hold still..."
        
        var allData: [UInt8] = []
        var currentBlock = 0
        
        func readNextBlock() {
            let message = NFCNDEFMessage(data: Data(allData))
            if let message = message, let record = message.records.first {
                if !record.payload.isEmpty {
                    print("payload: \(record.payload)")
                }
            }

            if currentBlock >= 800 {
                let bytes = allData
                print("bytes: \(bytes)")
                session.invalidate()
                return
            }
            
            print("current block: \(currentBlock)")
            
            tag.extendedReadMultipleBlocks(requestFlags: .highDataRate, blockRange: NSRange(location: currentBlock, length: blocksToRead)) { data, error in
                let result: Result<[Data], any Error> = {
                    if let error = error {
                        return .failure(error)
                    }
                    
                    return .success(data)
                }()
                
                switch result {
                case .success(let data):
                    for data in data {
                        print("\(currentBlock): \(data)")
                        let uint = [UInt8](data)
                        print("\(currentBlock) uint: \(uint)")
                        allData.append(contentsOf: uint)
                    }
                    
                    currentBlock = currentBlock + self.blocksToRead
                    
                    self.retries = 0
                    readNextBlock()
                case .failure(let error):
                    if self.retries < 10 {
                        Log.warn("read error: \(error.localizedDescription), retrying")
                        readNextBlock()
                        self.retries = self.retries + 1
                    } else {
                        Log.error("read error, retries exhausted: \(error.localizedDescription)")
                        self.tagReaderSession(session, didInvalidateWithError: error)
                    }
                }
            }
        }
        
        readNextBlock()
    }
    
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
    
    func tagReaderSessionDidBecomeActive(_ session: NFCTagReaderSession) {
        Log.debug("Tag reader session did become active.")
    }
    
    func tagReaderSession(_ session: NFCTagReaderSession, didInvalidateWithError error: any Error) {
        Log.error("Tag reader session did invalidate with error: \(error.localizedDescription)")
        switch error as? NFCReaderError {
        case .none:
            session.invalidate(errorMessage: "Unable to read NFC tag, try again")
        case .some(let error):
            switch error.code {
            case .readerTransceiveErrorTagConnectionLost:
                session.invalidate(errorMessage: "Tag connection lost, please hold your phone still")
            default:
                session.invalidate(errorMessage: "Unable to read NFC tag, try again")
            }
        }
    }
}
