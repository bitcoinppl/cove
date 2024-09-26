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
                self.readNDEF(from: iso15693Tag, session: session)
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
    
    func readNDEF<T: NFCNDEFTag>(from tag: T, session: NFCTagReaderSession) {
        Log.debug("reading NDEF message from tag: \(tag)")
        
        tag.readNDEF { message, error in
            if let error = error {
                if message == nil {
                    Log.error("read error: \(error.localizedDescription)")
                    session.invalidate(errorMessage: "Unable to read NFC tag please try again.")
                    session.restartPolling()
                    return
                }
            }
            
            guard let message = message else {
                Log.error("no NDEF message found")
                session.invalidate(errorMessage: "Unable to read NFC tag please try again.")
                session.restartPolling()
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
