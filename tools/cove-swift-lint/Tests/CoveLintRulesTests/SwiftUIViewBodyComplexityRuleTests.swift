@testable import CoveLintRules
import SwiftASTLintTestSupport
import Testing

struct SwiftUIViewBodyComplexityRuleTests {
    @Test("accepts a small SwiftUI body")
    func acceptsSmallBody() {
        let diagnostics = swiftUIViewBodyComplexityRule.lint(
            source: """
            import SwiftUI

            struct SmallView: View {
                var body: some View {
                    VStack {
                        Text("Title")
                        Text("Value")
                    }
                }
            }
            """
        )

        #expect(diagnostics.isEmpty)
    }

    @Test("reports nested view composition above the configured limit")
    func reportsNestedComposition() {
        let diagnostics = swiftUIViewBodyComplexityRule.lint(
            source: """
            import SwiftUI

            struct ComplexView: View {
                var body: some View {
                    VStack {
                        if true {
                            ForEach([1, 2, 3], id: \\.self) { value in
                                Button("Value \\(value)") {
                                    print(value)
                                }
                            }
                        }
                    }
                }
            }
            """,
            argsYAML: """
            maximumComplexity: 3
            severity: error
            """
        )

        #expect(diagnostics.count == 1)
        #expect(diagnostics[0].message.contains("complexity"))
    }

    @Test("includes local computed view helpers in the score")
    func followsComputedViewHelper() {
        let diagnostics = swiftUIViewBodyComplexityRule.lint(
            source: """
            import SwiftUI

            struct HelperView: View {
                var body: some View {
                    content
                }

                private var content: some View {
                    VStack {
                        if true {
                            Text("One")
                        }
                    }
                }
            }
            """,
            argsYAML: """
            maximumComplexity: 2
            severity: error
            """
        )

        #expect(diagnostics.count == 1)
    }

    @Test("includes local view functions in the score")
    func followsViewFunction() {
        let diagnostics = swiftUIViewBodyComplexityRule.lint(
            source: """
            import SwiftUI

            struct HelperView: View {
                var body: some View {
                    content()
                }

                private func content() -> some View {
                    VStack {
                        Text("One")
                    }
                }
            }
            """,
            argsYAML: """
            maximumComplexity: 2
            severity: error
            """
        )

        #expect(diagnostics.count == 1)
    }

    @Test("ignores computed properties on non-View types")
    func ignoresNonViewType() {
        let diagnostics = swiftUIViewBodyComplexityRule.lint(
            source: """
            struct Model {
                var body: some View {
                    makeValue {
                        if true {
                            value()
                        }
                    }
                }
            }
            """,
            argsYAML: """
            maximumComplexity: 1
            severity: error
            """
        )

        #expect(diagnostics.isEmpty)
    }
}
