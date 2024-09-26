//
//  NfcReader.swift
//  Cove
//
//  Created by Praveen Perera on 9/26/24.
//

import CoreNFC
import Foundation

@Observable
class NFCReader: NSObject, NFCNDEFReaderSessionDelegate {
    var scannedMessage: String? = nil
    private var session: NFCNDEFReaderSession?
    
    func scan() {
        session = NFCNDEFReaderSession(delegate: self, queue: nil, invalidateAfterFirstRead: false)
        session?.alertMessage = "Hold your iPhone near your hardware wallets NFC chip."
        session?.begin()
    }
    
    func readerSession(_ session: NFCNDEFReaderSession, didDetectNDEFs messages: [NFCNDEFMessage]) {
        guard let message = messages.first,
              let record = message.records.first,
              let payload = String(data: record.payload, encoding: .utf8)
        else {
            return
        }
        
        scannedMessage = payload
    }
    
    func readerSession(_ session: NFCNDEFReaderSession, didInvalidateWithError error: Error) {
        Log.error("NFCReader: \(error)")
    }
}
