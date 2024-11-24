//
//  NFCWriter.swift
//  Cove
//
//  Created by Praveen Perera on 11/24/24.
//

import CoreNFC
import SwiftUI

class NFCWriter: NSObject, NFCNDEFReaderSessionDelegate, ObservableObject {
    var text: String?
    var session: NFCNDEFReaderSession?
    
    func writeToTag(text: String) {
        self.text = text
        guard NFCNDEFReaderSession.readingAvailable else { return }
         
        session = NFCNDEFReaderSession(delegate: self, queue: nil, invalidateAfterFirstRead: false)
        session?.alertMessage = "Hold your iPhone near an NFC tag to write"
        session?.begin()
    }
    
    func readerSession(_ session: NFCNDEFReaderSession, didDetectNDEFs messages: [NFCNDEFMessage]) {}
    
    func readerSession(_ session: NFCNDEFReaderSession, didDetect tags: [NFCNDEFTag]) {
        guard let text, text.isEmpty == false else {
            session.invalidate(errorMessage: "No text to write to NFC tag")
            return
        }
        
        guard let tag = tags.first else {
            session.invalidate(errorMessage: "No tag found")
            return
        }
        
        session.connect(to: tag) { error in
            if let error {
                session.invalidate(errorMessage: error.localizedDescription)
                return
            }
            
            tag.queryNDEFStatus { _, _, error in
                guard error == nil else {
                    session.invalidate(errorMessage: "Failed to query tag")
                    return
                }
                
                let payload = NFCNDEFPayload.wellKnownTypeTextPayload(
                    string: text,
                    locale: Locale(identifier: "en")
                )!
                
                let message = NFCNDEFMessage(records: [payload])
                
                tag.writeNDEF(message) { error in
                    if let error {
                        session.invalidate(errorMessage: "Write failed: \(error.localizedDescription)")
                    } else {
                        session.alertMessage = "Successfully wrote to tag!"
                        session.invalidate()
                    }
                }
            }
        }
    }
    
    func readerSession(_ session: NFCNDEFReaderSession, didInvalidateWithError error: Error) {
        session.invalidate()
    }
}
