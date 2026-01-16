//
//  ColumnMajorGrid.swift
//  Cove
//
//  Created by Praveen Perera on 1/16/26.
//

import SwiftUI

/// A grid that displays items in column-major order (top-to-bottom, then left-to-right)
///
/// For a list [1,2,3,4,5,6] with 3 columns, displays as:
/// ```
/// 1  3  5
/// 2  4  6
/// ```
struct ColumnMajorGrid<Item, Content: View>: View {
    let items: [Item]
    let numberOfColumns: Int
    let spacing: CGFloat
    let content: (Int, Item) -> Content

    init(
        items: [Item],
        numberOfColumns: Int = 3,
        spacing: CGFloat = 12,
        @ViewBuilder content: @escaping (Int, Item) -> Content
    ) {
        self.items = items
        self.numberOfColumns = numberOfColumns
        self.spacing = spacing
        self.content = content
    }

    private var numberOfRows: Int {
        precondition(
            items.count % numberOfColumns == 0,
            "Item count (\(items.count)) must be divisible by \(numberOfColumns)"
        )
        return items.count / numberOfColumns
    }

    private var rows: [GridItem] {
        Array(repeating: GridItem(.flexible()), count: numberOfRows)
    }

    var body: some View {
        LazyHGrid(rows: rows, spacing: spacing) {
            ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                content(index, item)
            }
        }
    }
}
