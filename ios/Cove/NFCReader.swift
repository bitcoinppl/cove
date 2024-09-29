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
    
    func scan() {
        session = NFCTagReaderSession(pollingOption: [.iso14443, .iso15693], delegate: self)
        session?.alertMessage = "Hold your iPhone near the NFC tag."
        session?.begin()
    }
    
    func tagReaderSession(_ session: NFCTagReaderSession, didDetect tags: [NFCTag]) {
        print("tags count: \(tags.count)")
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
                self.readIso15693Tag(from: iso15693Tag, session: session)
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
    
    func readIso15693Tag(from tag: NFCISO15693Tag, session: NFCTagReaderSession) {
        print("reading iso tag large....")
        session.alertMessage = "Reading data please hold still..."
        readNDEF(from: tag, session: session)
//        tag.getSystemInfo(requestFlags: .highDataRate) { result in
//            switch result {
//            case .success(let systemInfo):
//                print("systemInfo: \(systemInfo)")
//                print("Block Size: \(systemInfo.blockSize)")
//                print("Total Blocks: \(systemInfo.totalBlocks)")
//
//            case .failure(let error):
//                print("Error getting system info: \(error.localizedDescription)")
//            }
//        }
//
//        readBlocks(tag: tag, session: session)
    }
    
    func readBlocks(tag: NFCISO15693Tag, session: NFCTagReaderSession) {
        print("read blocks....")
        var allData = Data()
        var currentBlock = 0
        
        func readNextBlock() {
            if currentBlock >= 255 {
                currentBlock = 0
            }
            
            print("current block: \(currentBlock)")
            
            tag.readSingleBlock(requestFlags: .highDataRate, blockNumber: UInt8(currentBlock)) { result in
                switch result {
                case .success(let data):
                    print("read data block: \(data.base64EncodedString())")
                    let bytes = [UInt8](data)
                    print("bytes: \(bytes)")
                    allData.append(data)
                    currentBlock += 1
                    readNextBlock()
                case .failure(let error):
                    print("Error reading block \(currentBlock): \(error.localizedDescription)")
                    session.invalidate(errorMessage: "Error reading tag")
                }
            }
        }
        
        readNextBlock()
    }

//
//    func readAllBlocks(tag: NFCISO15693Tag, blockSize: Int, blockCount: Int, session: NFCTagReaderSession) {
//        var data = Data()
//        let group = DispatchGroup()
//
//        for i in 0 ..< blockCount {
//            group.enter()
//            tag.readSingleBlock(requestFlags: .highDataRate, blockNumber: UInt8(i)) { blockData, error in
//                defer { group.leave() }
//                guard let blockData = blockData, error == nil else {
//                    return
//                }
//                data.append(blockData)
//            }
//        }
//
//        group.notify(queue: .main) {
    ////            self.processTagData(data)
//            session.alertMessage = "Tag read successfully!"
//            session.invalidate()
//        }
//    }

    func readNDEF<T: NFCNDEFTag>(from tag: T, session: NFCTagReaderSession) {
        Log.debug("reading NDEF message from tag: \(tag)")
        
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
    }
}
