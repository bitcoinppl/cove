import SwiftASTLint
import SwiftSyntax

struct SwiftUIViewBodyComplexityArguments: Codable, Sendable {
    var maximumComplexity = 10
    var severity = Severity.error
}

let swiftUIViewBodyComplexityRule = ParameterizedRule(
    id: "swiftui-view-body-complexity",
    description: "Limits SwiftUI view composition across body declarations and local view helpers",
    defaultArguments: SwiftUIViewBodyComplexityArguments()
) { file, context, arguments in
    let finder = SwiftUIViewFinder()
    finder.walk(file)

    for declaration in finder.declarations {
        let measurement = ViewBodyComplexityMeasurer(declaration: declaration).measure()
        guard measurement.score > arguments.maximumComplexity else { continue }

        context.report(
            on: declaration.bodyVariable,
            message: """
            SwiftUI view body complexity is \(measurement.score); \
            maximum allowed complexity is \(arguments.maximumComplexity). \
            Extract dedicated View types.
            """,
            severity: arguments.severity
        )
    }
}

private struct SwiftUIViewDeclaration {
    let bodyVariable: VariableDeclSyntax
    let bodySyntax: Syntax
    let helpers: [String: Syntax]
}

private final class SwiftUIViewFinder: SyntaxVisitor {
    private(set) var declarations: [SwiftUIViewDeclaration] = []

    init() {
        super.init(viewMode: .sourceAccurate)
    }

    override func visit(_ node: StructDeclSyntax) -> SyntaxVisitorContinueKind {
        collect(from: node)
        return .visitChildren
    }

    override func visit(_ node: ClassDeclSyntax) -> SyntaxVisitorContinueKind {
        collect(from: node)
        return .visitChildren
    }

    override func visit(_ node: EnumDeclSyntax) -> SyntaxVisitorContinueKind {
        collect(from: node)
        return .visitChildren
    }

    override func visit(_ node: ExtensionDeclSyntax) -> SyntaxVisitorContinueKind {
        collect(from: node)
        return .visitChildren
    }

    private func collect(from declaration: some DeclGroupSyntax) {
        guard declaration.inheritanceClause?.inheritsView == true else { return }

        let members = declaration.memberBlock.members
        guard let body = members.compactMap(\.decl.asVariable).first(where: \.isViewBody) else {
            return
        }
        guard let bodySyntax = body.viewBodySyntax else { return }

        let helpers = members.reduce(into: [String: Syntax]()) { result, member in
            if let variable = member.decl.as(VariableDeclSyntax.self),
               let helper = variable.viewHelper
            {
                result[helper.name] = helper.syntax
            }

            if let function = member.decl.as(FunctionDeclSyntax.self),
               let helper = function.viewHelper
            {
                result[helper.name] = helper.syntax
            }
        }

        declarations.append(
            SwiftUIViewDeclaration(
                bodyVariable: body,
                bodySyntax: bodySyntax,
                helpers: helpers
            )
        )
    }
}

private struct ViewBodyComplexityMeasurement {
    let score: Int
}

private final class ViewBodyComplexityMeasurer {
    private let declaration: SwiftUIViewDeclaration
    private var expandedHelpers = Set<String>()

    init(declaration: SwiftUIViewDeclaration) {
        self.declaration = declaration
    }

    func measure() -> ViewBodyComplexityMeasurement {
        let score = 1 + measure(declaration.bodySyntax)
        return ViewBodyComplexityMeasurement(score: score)
    }

    private func measure(_ syntax: Syntax) -> Int {
        let visitor = CompositionVisitor(helperNames: Set(declaration.helpers.keys))
        visitor.walk(syntax)

        var score = visitor.score
        for reference in visitor.helperReferences {
            score += 1
            guard expandedHelpers.insert(reference).inserted,
                  let helper = declaration.helpers[reference]
            else {
                continue
            }

            score += measure(helper)
        }

        return score
    }
}

private final class CompositionVisitor: SyntaxVisitor {
    private let helperNames: Set<String>

    private(set) var score = 0
    private(set) var helperReferences: [String] = []

    init(helperNames: Set<String>) {
        self.helperNames = helperNames
        super.init(viewMode: .sourceAccurate)
    }

    override func visit(_ node: ClosureExprSyntax) -> SyntaxVisitorContinueKind {
        score += 1
        return .visitChildren
    }

    override func visit(_ node: IfExprSyntax) -> SyntaxVisitorContinueKind {
        score += 1
        return .visitChildren
    }

    override func visit(_ node: SwitchCaseSyntax) -> SyntaxVisitorContinueKind {
        score += 1
        return .visitChildren
    }

    override func visit(_ node: ForStmtSyntax) -> SyntaxVisitorContinueKind {
        score += 1
        return .visitChildren
    }

    override func visit(_ node: WhileStmtSyntax) -> SyntaxVisitorContinueKind {
        score += 1
        return .visitChildren
    }

    override func visit(_ node: RepeatStmtSyntax) -> SyntaxVisitorContinueKind {
        score += 1
        return .visitChildren
    }

    override func visit(_ node: TernaryExprSyntax) -> SyntaxVisitorContinueKind {
        score += 1
        return .visitChildren
    }

    override func visit(_ node: DeclReferenceExprSyntax) -> SyntaxVisitorContinueKind {
        let name = node.baseName.text
        if helperNames.contains(name) {
            helperReferences.append(name)
        }

        return .visitChildren
    }
}

private extension InheritanceClauseSyntax {
    var inheritsView: Bool {
        inheritedTypes.contains { inheritedType in
            let name = inheritedType.type.trimmedDescription
            return name == "View" || name == "SwiftUI.View"
        }
    }
}

private extension DeclSyntax {
    var asVariable: VariableDeclSyntax? {
        `as`(VariableDeclSyntax.self)
    }
}

private extension VariableDeclSyntax {
    var isViewBody: Bool {
        bindings.contains { binding in
            binding.identifier == "body" && binding.typeAnnotation?.type.isSomeView == true
        }
    }

    var viewBodySyntax: Syntax? {
        bindings.first { $0.identifier == "body" }?
            .accessorBlock
            .map(Syntax.init)
    }

    var viewHelper: (name: String, syntax: Syntax)? {
        guard let binding = bindings.first,
              binding.typeAnnotation?.type.isSomeView == true,
              let name = binding.identifier,
              name != "body",
              let accessorBlock = binding.accessorBlock
        else {
            return nil
        }

        return (name, Syntax(accessorBlock))
    }
}

private extension FunctionDeclSyntax {
    var viewHelper: (name: String, syntax: Syntax)? {
        guard signature.returnClause?.type.isSomeView == true,
              let body
        else {
            return nil
        }

        return (name.text, Syntax(body))
    }
}

private extension PatternBindingSyntax {
    var identifier: String? {
        pattern.as(IdentifierPatternSyntax.self)?.identifier.text
    }
}

private extension TypeSyntax {
    var isSomeView: Bool {
        let description = trimmedDescription
        return description == "some View" || description == "some SwiftUI.View"
    }
}
