//
//  NodeSelectionView.swift
//  Cove
//
//  Created by Praveen Perera on 7/18/24.
//

import SwiftUI

struct NodeSelectionView: View {
    let nodeSelector = NodeSelector()

    @State private var selectedNodeName: String
    private var nodeList: [NodeSelection]

    @State private var customUrl: String = ""

    init() {
        self.selectedNodeName = nodeSelector.selectedNode().name
        self.nodeList = nodeSelector.nodeList()
    }

    var showCustomUrlField: Bool {
        selectedNodeName == "Custom"
    }

    var body: some View {
        Section(header: Text("Node Selection")) {
            Picker("Select Node", selection: $selectedNodeName) {
                ForEach(nodeList, id: \.name) { (node: NodeSelection) in
                    Text(node.name)
                        .tag(node.name)
                }

                Text("Custom").tag("Custom")
            }
            if showCustomUrlField {
                TextField("Enter custom node URL", text: $customUrl)

                Button("Save Custom Node") {
                    // Update app state with custom node
//                        app.dispatch(action: .selectNode(node: Nodejk(name: "Custom", url: customURL)))
                }
            }
        }
        .onChange(of: selectedNodeName) { old, new in
            print(old, new)
        }
    }
}

#Preview {
    NodeSelectionView()
}
