//
//  NodeSelection+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 7/21/24.
//

import Foundation

extension NodeSelection {
    var url: String {
        toNode().url
    }

    var name: String {
        toNode().name
    }
}
