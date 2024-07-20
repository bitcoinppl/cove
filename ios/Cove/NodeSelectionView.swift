//
//  NodeSelectionView.swift
//  Cove
//
//  Created by Praveen Perera on 7/18/24.
//

import SwiftUI

struct NodeSelectionView: View {
    let app: MainViewModel
    let nodeSelector: NodeSelector = .init()

    @State
    @State private var customURL: String = ""
    @State private var showCustomURLField: Bool = false

    var body: some View {
        Section(header: Text("Node Selection")) {
            Picker("Select Node",
                   selection: Binding(
                       get: { app.selectedNode },
                       set: { app.dispatch(action: .setSelectedNode($0)) }
                   )) {
                ForEach(nodeSelector.nodeList(), id: \.self) { node in
                    Text(node.name)
                        .tag(node)
                }
                Text("Custom").tag("Custom")
            }
            .onChange(of: app.selectedNode) { _, newValue in
                showCustomURLField = (newValue == "Custom")
                if newValue != "Custom" {
                    customURL = ""
                    // Update app state with selected node
//                        app.dispatch(action: .selectNode(node: Node(name: newValue, url: "")))
                }
            }

            if showCustomURLField {
                TextField("Enter custom node URL", text: $customURL)

                Button("Save Custom Node") {
                    // Update app state with custom node
//                        app.dispatch(action: .selectNode(node: Node(name: "Custom", url: customURL)))
                }
            }
        }
    }
}

#Preview {
    NodeSelectionView()
}
