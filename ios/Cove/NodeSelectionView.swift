//
//  NodeSelectionView.swift
//  Cove
//
//  Created by Praveen Perera on 7/18/24.
//

import SwiftUI

struct NodeSelectionView: View {
    let nodeSelector = NodeSelector()

    @State private var selectedNode: NodeSelection?
    @State private var customUrl: String = ""
    @State private var showCustomUrlField: Bool = false

    var body: some View {
        Section(header: Text("Node Selection")) {
            Picker("Select Node", selection: $selectedNode) {
                ForEach(nodeSelector.nodeList(), id: \.url) { (node: NodeSelection) in
                    Text(node.name)
                        .tag(node.url)
                }
                Text("Custom").tag("Custom")
            }
            .onChange(of: selectedNode) { _, newSelectedNode in
                if case let .custom(node) = newSelectedNode {
                    customUrl = node.url
                    showCustomUrlField = true
                }
            }

            if showCustomUrlField {
                TextField("Enter custom node URL", text: $customUrl)

                Button("Save Custom Node") {
                    // Update app state with custom node
//                        app.dispatch(action: .selectNode(node: Node(name: "Custom", url: customURL)))
                }
            }
        }
        .onAppear {
            selectedNode = nodeSelector.selectedNode()
        }
    }
}

#Preview {
    NodeSelectionView()
}
