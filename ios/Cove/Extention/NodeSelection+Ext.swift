//
//  NodeSelection+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 7/21/24.
//

import Foundation

extension NodeSelection {
    var node: Node {
        nodeSelectionToNode(node: self)
    }

    var url: String {
        node.url
    }

    var name: String {
        node.name
    }
}
