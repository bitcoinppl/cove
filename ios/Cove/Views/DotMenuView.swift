//
//  DotMenuView.swift
//  Cove
//
//  Created by Praveen Perera on 12/1/24.
//
import SwiftUI

struct DotMenuView: View {
    var selected: Int = 1
    var size: CGFloat = 20
    var total: Int = 4
    var spacing: CGFloat = 12

    @State private var isSelected = false

    var body: some View {
        HStack(spacing: spacing) {
            ForEach(0 ..< total, id: \.self) { index in
                if index == selected {
                    Capsule()
                        .fill(.white)
                        .frame(width: size * 3, height: size)
                        .onTapGesture {
                            withAnimation {
                                isSelected.toggle()
                            }
                        }
                } else {
                    Circle()
                        .fill(.coveLightGray.opacity(0.5))
                        .frame(width: size, height: size)
                }
            }
        }
    }
}

#Preview("cadences") {
    VStack(spacing: 16) {
        DotMenuView(selected: 0)
        DotMenuView(selected: 1)
        DotMenuView(selected: 2)
        DotMenuView(selected: 3)
    }
}

#Preview("sizes") {
    VStack {
        DotMenuView(size: 5)
        DotMenuView(size: 10)
        DotMenuView(size: 20)
        DotMenuView(size: 30)
        DotMenuView(size: 50)
    }
}

#Preview("number") {
    DotMenuView(selected: 0, total: 6)
    DotMenuView(selected: 1, total: 6)
    DotMenuView(selected: 0, total: 8)
    DotMenuView(selected: 1, total: 8)
    DotMenuView(selected: 4, total: 10)
    DotMenuView(selected: 8, total: 10)
}
