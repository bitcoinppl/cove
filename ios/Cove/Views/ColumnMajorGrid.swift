//
//  ColumnMajorGrid.swift
//  Cove
//
//  Created by Praveen Perera on 1/16/26.
//

import SwiftUI

/// A grid that displays items in column-major order (top-to-bottom, then left-to-right)
///
/// For a list [1,2,3,4,5,6] with 2 columns, displays as:
/// ```
/// 1  4
/// 2  5
/// 3  6
/// ```
struct ColumnMajorGrid<Item, Content: View>: View {
    let items: [Item]
    let numberOfColumns: Int
    let spacing: CGFloat
    let content: (Int, Item) -> Content

    init(
        items: [Item],
        numberOfColumns: Int = 2,
        spacing: CGFloat = 12,
        @ViewBuilder content: @escaping (Int, Item) -> Content
    ) {
        self.items = items
        self.numberOfColumns = numberOfColumns
        self.spacing = spacing
        self.content = content
    }

    private var itemsPerColumn: Int {
        precondition(
            items.count % numberOfColumns == 0,
            "Item count (\(items.count)) must be divisible by \(numberOfColumns)"
        )
        return items.count / numberOfColumns
    }

    var body: some View {
        HStack(alignment: .top, spacing: spacing) {
            ForEach(0 ..< numberOfColumns, id: \.self) { col in
                VStack(spacing: spacing) {
                    ForEach(0 ..< itemsPerColumn, id: \.self) { row in
                        let index = col * itemsPerColumn + row
                        content(index, items[index])
                    }
                }
                .frame(maxWidth: .infinity)
            }
        }
    }
}
