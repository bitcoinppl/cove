import Foundation
import PDFKit
import SwiftUI

struct ColorView: View {
    @State private var contentSize: CGSize = .zero

    func savePdf() {
        // Create a custom view with the appropriate size
        let pdfView = ScrollableContentView(content: self, contentSize: contentSize)
        let renderer = ImageRenderer(content: pdfView)

        let docsDir = URL.documentsDirectory
        let url = docsDir.appending(path: "color_sheet.pdf")

        renderer.render { size, context in
            var box = CGRect(x: 0, y: 0, width: size.width, height: max(size.height, contentSize.height))
            guard let pdf = CGContext(url as CFURL, mediaBox: &box, nil) else {
                return
            }

            pdf.beginPDFPage(nil)
            context(pdf)
            pdf.endPDFPage()
            pdf.closePDF()
        }

        print("open \(url)")
        print("open \(docsDir)")
    }

    func savePrintablePdf() {
        let docsDir = URL.documentsDirectory
        let url = docsDir.appending(path: "color_sheet_printable.pdf")

        // Define page size (A4)
        let pageWidth: CGFloat = 595.2
        let pageHeight: CGFloat = 841.8
        let pageRect = CGRect(x: 0, y: 0, width: pageWidth, height: pageHeight)

        // Setup PDF renderer
        let format = UIGraphicsPDFRendererFormat()
        let renderer = UIGraphicsPDFRenderer(bounds: pageRect, format: format)

        // Create groups of colors for pagination
        let colorsPerPage = 10 // Adjust based on how many fit on one page
        let colorGroups = stride(from: 0, to: LabeledColor.allColors.count, by: colorsPerPage).map {
            Array(LabeledColor.allColors[$0 ..< min($0 + colorsPerPage, LabeledColor.allColors.count)])
        }

        // Write the PDF
        var pageNumber = 0
        try? renderer.writePDF(to: url) { context in
            for colorGroup in colorGroups {
                // Begin a new page
                context.beginPage()
                pageNumber += 1

                if pageNumber == 1 {
                    // Create header
                    let titleAttributes: [NSAttributedString.Key: Any] = [.font: UIFont.boldSystemFont(ofSize: 16)]
                    let title = "Color Palette"
                    title.draw(at: CGPoint(x: 30, y: 30), withAttributes: titleAttributes)
                }

                // Create each page content
                let pageView = VStack {
                    TableContent(colors: colorGroup, columns: 1)
                    Spacer()
                }
                let hostingController = UIHostingController(rootView: pageView)
                hostingController.view.frame = CGRect(
                    x: 20,
                    y: 60,
                    width: pageWidth - 40,
                    height: pageHeight - 80
                )

                // Render the view
                let targetView = hostingController.view!
                targetView.setNeedsLayout()
                targetView.layoutIfNeeded()

                let renderer = UIGraphicsImageRenderer(bounds: targetView.bounds)
                let image = renderer.image { _ in
                    targetView.drawHierarchy(in: targetView.bounds, afterScreenUpdates: true)
                }

                image.draw(in: hostingController.view.frame)
            }
        }

        print("open \(url)")
        print("open \(docsDir)")
    }

    /// Helper view for each page of colors
    struct ColorPageView: View {
        let colors: [LabeledColor]

        var body: some View {
            VStack(spacing: 10) {
                HStack {
                    Text("Color Name")
                        .fontWeight(.bold)
                    Spacer()
                    Text("Light Mode")
                        .frame(width: 75)
                    Text("Dark Mode")
                        .frame(width: 75)
                }
                .font(.footnote)

                ForEach(colors, id: \.name) { color in
                    VStack {
                        HStack {
                            Text(color.name)
                                .lineLimit(1)
                                .minimumScaleFactor(0.3)

                            Spacer()

                            VStack {
                                Rectangle()
                                    .frame(width: 75, height: 30)
                                    .foregroundColor(color.color)
                                    .border(Color.black, width: 2)

                                Text(color.color.toHexStringAndOpacity(colorScheme: .light))
                                    .font(.caption2)
                                    .fontWeight(.semibold)
                                    .foregroundStyle(.secondary)
                            }
                            .environment(\.colorScheme, .light)

                            VStack {
                                Rectangle()
                                    .frame(width: 75, height: 30)
                                    .foregroundColor(color.color)
                                    .border(Color.black, width: 2)

                                Text(color.color.toHexStringAndOpacity(colorScheme: .dark))
                                    .font(.caption2)
                                    .fontWeight(.semibold)
                                    .foregroundStyle(.gray)
                            }
                            .environment(\.colorScheme, .dark)
                        }
                        Divider()
                    }
                }
            }
        }
    }

    var body: some View {
        VStack(alignment: .center, spacing: 0) {
            HStack {
                if contentSize != .zero {
                    Button("Export PDF") { savePdf() }
                        .frame(width: 300)

                    Button("Export Printable PDF") { savePrintablePdf() }
                        .frame(width: 300)
                }
            }
            .font(.footnote)
            .fontWeight(.bold)
            .padding()

            ScrollView {
                TableContent(colors: LabeledColor.allColors)
                    .background(
                        GeometryReader { geo in
                            Color.clear.onAppear {
                                contentSize = geo.size
                            }
                        }
                    )
            }
            .padding()
        }
    }
}

/// This view will render the full content without scrolling
struct ScrollableContentView: View {
    let content: ColorView
    let contentSize: CGSize

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Color Palette")
                    .font(.headline)
            }
            .font(.footnote)
            .fontWeight(.bold)
            .padding()

            // Manually render all color items without ScrollView
            TableContent(colors: LabeledColor.allColors)
        }
        // Use the measured content size for layout
        .frame(width: contentSize.width, height: contentSize.height + 100) // Add padding for header/footer
    }
}

private let spacing: CGFloat = 30
struct TableContent: View {
    let colors: [LabeledColor]
    var columns = 2

    /// Define grid layout with 2 columns
    func columnsDef(_ c: Int) -> [GridItem] {
        Array(repeating: GridItem(.flexible(), spacing: spacing), count: c)
    }

    var body: some View {
        LazyVGrid(columns: columnsDef(columns), spacing: 16) {
            ForEach(colors, id: \.name) { color in
                ColorRow(color: color)
            }
        }
        .padding()
    }
}

#Preview {
    ColorView()
}

struct ColorRow: View {
    let color: LabeledColor

    var body: some View {
        VStack {
            HStack {
                HStack {
                    Text(color.name)
                        .lineLimit(1)
                        .minimumScaleFactor(0.3)
                }

                Spacer()

                if color.color.hasDarkVariant {
                    HStack {
                        VStack {
                            Rectangle()
                                .frame(width: 75, height: 30)
                                .foregroundColor(color.color)
                                .border(Color.black, width: 3)

                            HStack(spacing: 0) {
                                Text(color.color.toHexStringAndOpacity(colorScheme: .light))
                                    .font(.caption2)
                                    .fontWeight(.semibold)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .environment(\.colorScheme, .light)

                        VStack {
                            Rectangle()
                                .frame(width: 75, height: 30)
                                .foregroundColor(color.color)
                                .border(Color.black, width: 3)

                            HStack(spacing: 0) {
                                Text(color.color.toHexStringAndOpacity(colorScheme: .dark))
                                    .font(.caption2)
                                    .fontWeight(.semibold)
                                    .foregroundStyle(.gray)
                            }
                        }
                        .environment(\.colorScheme, .dark)
                    }
                }

                if !color.color.hasDarkVariant {
                    VStack {
                        Rectangle()
                            .frame(width: 75 * 2 + 10, height: 30)
                            .foregroundColor(color.color)
                            .border(Color.black, width: 3)

                        HStack(spacing: 0) {
                            Text(color.color.toHexStringAndOpacity(colorScheme: .light))
                                .font(.caption2)
                                .fontWeight(.semibold)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .environment(\.colorScheme, .light)
                }
            }
            Divider()
        }
        .padding(.vertical, 2)
        .font(.caption)
    }
}
