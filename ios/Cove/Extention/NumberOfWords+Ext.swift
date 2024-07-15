//
//  NumberOfWords+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 7/15/24.
//

import Foundation

extension NumberOfBip39Words {
    func toWordCount() -> Int {
        Int(numberOfWordsToWordCount(me: self))
    }

    func inGroups(of groupSize: Int = 6) -> [[String]] {
        numberOfWordsInGroups(me: self, of: UInt8(groupSize))
    }
}
