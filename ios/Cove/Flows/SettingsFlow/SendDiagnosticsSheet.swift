import SwiftUI
import UIKit

private let diagnosticsFilename = "cove-diagnostics.txt"
private let previewChunkSize = 4096
private let previewRefreshDelayNanoseconds: UInt64 = 300_000_000

private struct DiagnosticsPreviewChunk: Identifiable {
    let id: Int
    let text: String
}

private enum DiagnosticsLoadState: Equatable {
    case loading
    case ready
    case failed(String)
}

private enum SendDiagnosticsAlert: Identifiable, Equatable {
    case confirmClear
    case error(String)

    var id: String {
        switch self {
        case .confirmClear:
            "confirm-clear"
        case let .error(message):
            "error-\(message)"
        }
    }
}

struct SendDiagnosticsSheet: View {
    @Environment(\.dismiss) private var dismiss

    @State private var report: DiagnosticsReport? = nil
    @State private var previewText = ""
    @State private var previewChunks: [DiagnosticsPreviewChunk] = []
    @State private var description = ""
    @State private var reportSize = ""
    @State private var reportId: String? = nil
    @State private var submissionWarning: String? = nil
    @State private var loadState = DiagnosticsLoadState.loading
    @State private var isSubmitting = false
    @State private var alertState: SendDiagnosticsAlert? = nil
    @State private var previewRefreshTask: Task<Void, Never>? = nil

    private var isReady: Bool {
        switch loadState {
        case .ready:
            true
        case .loading, .failed:
            false
        }
    }

    private var exportText: String {
        report?.previewTextForDescription(description: description) ?? previewText
    }

    var body: some View {
        SendDiagnosticsNavigationContent(
            loadState: loadState,
            description: $description,
            previewChunks: previewChunks,
            reportSize: reportSize,
            reportId: reportId,
            submissionWarning: submissionWarning,
            isReady: isReady,
            isSubmitting: isSubmitting,
            canSubmit: report != nil && reportId == nil,
            retry: retryBuildReport,
            share: shareDiagnostics,
            clear: { alertState = .confirmClear },
            submit: submitReport,
            done: { dismiss() }
        )
        .task {
            if report == nil {
                await rebuildReport(clearStoredLogs: false)
            }
        }
        .onChange(of: description) { _, _ in
            schedulePreviewRefresh()
        }
        .onDisappear {
            previewRefreshTask?.cancel()
        }
        .alert(item: $alertState) { alert in
            diagnosticsAlert(alert)
        }
        .interactiveDismissDisabled(isSubmitting)
    }

    private func retryBuildReport() {
        Task { await rebuildReport(clearStoredLogs: false) }
    }

    private func diagnosticsAlert(_ alert: SendDiagnosticsAlert) -> Alert {
        switch alert {
        case .confirmClear:
            Alert(
                title: Text("Clear Stored Logs?"),
                message: Text("This deletes stored diagnostics logs on this device and rebuilds the preview."),
                primaryButton: .destructive(Text("Clear")) {
                    Task { await rebuildReport(clearStoredLogs: true) }
                },
                secondaryButton: .cancel()
            )

        case let .error(message):
            Alert(
                title: Text("Something went wrong"),
                message: Text(message),
                dismissButton: .default(Text("OK"))
            )
        }
    }

    @MainActor
    private func rebuildReport(clearStoredLogs: Bool) async {
        loadState = .loading
        reportId = nil
        submissionWarning = nil
        report = nil
        previewRefreshTask?.cancel()
        previewRefreshTask = nil
        previewText = ""
        previewChunks = []
        reportSize = ""

        do {
            if clearStoredLogs {
                try clearDiagnosticsLogs()
                try SwiftLogStore.shared.clear()
            }

            let nextReport = try await buildDiagnosticsReport(
                platform: IOSDiagnostics.platformInfo(),
                platformLogs: IOSDiagnostics.platformLogs()
            )

            report = nextReport
            refreshPreview(report: nextReport)
            loadState = .ready
        } catch {
            loadState = .failed(diagnosticsErrorMessage(error))
        }
    }

    @MainActor
    private func submitReport() async {
        guard let report else { return }

        isSubmitting = true
        defer { isSubmitting = false }

        do {
            let submission = try await report.submit(description: description)
            reportId = submission.reportId
            submissionWarning = submission.warning
        } catch {
            alertState = .error(diagnosticsErrorMessage(error))
        }
    }

    @MainActor
    private func shareDiagnostics() {
        ShareSheet.present(data: exportText, filename: diagnosticsFilename) { success in
            if !success { Log.warn("Diagnostics share cancelled or failed") }
        }
    }

    @MainActor
    private func refreshPreviewForCurrentDescription() {
        guard let report else { return }

        refreshPreview(report: report)
    }

    @MainActor
    private func schedulePreviewRefresh() {
        previewRefreshTask?.cancel()
        previewRefreshTask = Task {
            try? await Task.sleep(nanoseconds: previewRefreshDelayNanoseconds)
            guard !Task.isCancelled else { return }

            await MainActor.run {
                refreshPreviewForCurrentDescription()
            }
        }
    }

    @MainActor
    private func refreshPreview(report: DiagnosticsReport) {
        let nextPreviewText = report.previewTextForDescription(description: description)

        previewText = nextPreviewText
        previewChunks = Self.chunks(for: nextPreviewText)
        reportSize = report.formattedSizeForDescription(description: description)
    }

    private static func chunks(for text: String) -> [DiagnosticsPreviewChunk] {
        var chunks: [DiagnosticsPreviewChunk] = []
        var start = text.startIndex
        var chunkId = 0

        while start < text.endIndex {
            let end = text.index(start, offsetBy: previewChunkSize, limitedBy: text.endIndex) ?? text.endIndex
            chunks.append(DiagnosticsPreviewChunk(id: chunkId, text: String(text[start ..< end])))
            start = end
            chunkId += 1
        }

        return chunks
    }
}

private struct SendDiagnosticsNavigationContent: View {
    let loadState: DiagnosticsLoadState
    @Binding var description: String
    let previewChunks: [DiagnosticsPreviewChunk]
    let reportSize: String
    let reportId: String?
    let submissionWarning: String?
    let isReady: Bool
    let isSubmitting: Bool
    let canSubmit: Bool
    let retry: () -> Void
    let share: () -> Void
    let clear: () -> Void
    let submit: () async -> Void
    let done: () -> Void

    var body: some View {
        NavigationStack {
            SendDiagnosticsContent(
                loadState: loadState,
                description: $description,
                previewChunks: previewChunks,
                reportSize: reportSize,
                reportId: reportId,
                submissionWarning: submissionWarning,
                isReady: isReady,
                isSubmitting: isSubmitting,
                canSubmit: canSubmit,
                retry: retry,
                share: share,
                clear: clear,
                submit: submit,
                done: done
            )
            .navigationTitle("Send Diagnostics")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done", action: done)
                        .disabled(isSubmitting)
                }
            }
        }
    }
}

private struct SendDiagnosticsContent: View {
    private let content: AnyView

    init(
        loadState: DiagnosticsLoadState,
        description: Binding<String>,
        previewChunks: [DiagnosticsPreviewChunk],
        reportSize: String,
        reportId: String?,
        submissionWarning: String?,
        isReady: Bool,
        isSubmitting: Bool,
        canSubmit: Bool,
        retry: @escaping () -> Void,
        share: @escaping () -> Void,
        clear: @escaping () -> Void,
        submit: @escaping () async -> Void,
        done: @escaping () -> Void
    ) {
        content = switch loadState {
        case .loading:
            AnyView(
                ProgressView("Building diagnostics...")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            )
        case let .failed(message):
            AnyView(SendDiagnosticsFailureView(message: message, retry: retry))
        case .ready:
            AnyView(SendDiagnosticsReadyContent(
                description: description,
                previewChunks: previewChunks,
                reportSize: reportSize,
                reportId: reportId,
                submissionWarning: submissionWarning,
                isReady: isReady,
                isSubmitting: isSubmitting,
                canSubmit: canSubmit,
                share: share,
                clear: clear,
                submit: submit,
                done: done
            ))
        }
    }

    var body: some View {
        content
    }
}

private struct SendDiagnosticsFailureView: View {
    let message: String
    let retry: () -> Void

    var body: some View {
        VStack(spacing: 16) {
            Text("Diagnostics Unavailable")
                .font(.headline)
            Text(message)
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Button("Retry", action: retry)
                .buttonStyle(.borderedProminent)
        }
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct SendDiagnosticsReadyContent: View {
    @Binding var description: String
    let previewChunks: [DiagnosticsPreviewChunk]
    let reportSize: String
    let reportId: String?
    let submissionWarning: String?
    let isReady: Bool
    let isSubmitting: Bool
    let canSubmit: Bool
    let share: () -> Void
    let clear: () -> Void
    let submit: () async -> Void
    let done: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            SendDiagnosticsDescriptionEditor(description: $description)
            SendDiagnosticsPreviewHeader(reportSize: reportSize)
            SendDiagnosticsPreview(chunks: previewChunks)

            if let reportId {
                SendDiagnosticsSubmittedCard(
                    reportId: reportId,
                    warning: submissionWarning,
                    done: done
                )
            }

            SendDiagnosticsActions(
                isReady: isReady,
                isSubmitting: isSubmitting,
                share: share,
                clear: clear
            )
            SendDiagnosticsSubmitButton(
                isSubmitting: isSubmitting,
                isDisabled: !canSubmit || isSubmitting,
                submit: submit
            )
        }
        .padding()
    }
}

private struct SendDiagnosticsDescriptionEditor: View {
    @Binding var description: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Description")
                .font(.headline)
            TextEditor(text: $description)
                .frame(minHeight: 84, maxHeight: 120)
                .padding(8)
                .background(Color(.secondarySystemBackground))
                .clipShape(RoundedRectangle(cornerRadius: 8))
        }
    }
}

private struct SendDiagnosticsPreviewHeader: View {
    let reportSize: String

    var body: some View {
        HStack {
            Text("Preview")
                .font(.headline)
            Spacer()

            if !reportSize.isEmpty {
                Text(reportSize)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

private struct SendDiagnosticsPreview: View {
    let chunks: [DiagnosticsPreviewChunk]

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(chunks) { chunk in
                    Text(chunk.text)
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(.primary)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(12)
        }
        .background(Color(.secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}

private struct SendDiagnosticsSubmittedCard: View {
    let reportId: String
    let warning: String?
    let done: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Diagnostics sent")
                .font(.headline)
            Text(reportId)
                .font(.system(.callout, design: .monospaced))
                .textSelection(.enabled)

            if let warning {
                Text(warning)
                    .font(.footnote)
                    .foregroundStyle(.red)
            }

            HStack {
                Button("Copy ID", action: copyId)
                    .buttonStyle(.bordered)
                Button("Done", action: done)
                    .buttonStyle(.borderedProminent)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(.secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    private func copyId() {
        UIPasteboard.general.string = reportId
    }
}

private struct SendDiagnosticsActions: View {
    let isReady: Bool
    let isSubmitting: Bool
    let share: () -> Void
    let clear: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Button("Share", action: share)
                .buttonStyle(.bordered)
                .disabled(!isReady || isSubmitting)
            Button("Clear Stored Logs", role: .destructive, action: clear)
                .buttonStyle(.bordered)
                .disabled(isSubmitting)
        }
    }
}

private struct SendDiagnosticsSubmitButton: View {
    let isSubmitting: Bool
    let isDisabled: Bool
    let submit: () async -> Void

    var body: some View {
        Button {
            Task { await submit() }
        } label: {
            if isSubmitting {
                ProgressView()
                    .frame(maxWidth: .infinity)
            } else {
                Text("Submit")
                    .frame(maxWidth: .infinity)
            }
        }
        .buttonStyle(.borderedProminent)
        .disabled(isDisabled)
    }
}

private func diagnosticsErrorMessage(_ error: Error) -> String {
    guard let diagnosticsError = error as? DiagnosticsError else {
        return error.localizedDescription
    }

    switch diagnosticsError {
    case let .Build(message):
        return message
    case let .ClearLogs(message):
        return message
    case let .Submit(message):
        return message
    }
}

private enum IOSDiagnostics {
    static func platformInfo() -> DiagnosticsPlatformInfo {
        DiagnosticsPlatformInfo(
            platform: "iOS",
            buildNumber: bundleValue("CFBundleVersion"),
            osVersion: UIDevice.current.systemVersion,
            deviceModel: deviceModelIdentifier()
        )
    }

    static func platformLogs() -> String {
        let swiftLogs = SwiftLogStore.shared.snapshot()

        return [
            "iOS system logs are unavailable to sandboxed apps; app-recorded Swift logs are below.",
            "Generated: \(ISO8601DateFormatter().string(from: Date()))",
            "App version: \(bundleValue("CFBundleShortVersionString"))",
            "Build: \(bundleValue("CFBundleVersion"))",
            "iOS: \(UIDevice.current.systemVersion)",
            "Device: \(deviceModelIdentifier())",
            "Low power mode: \(ProcessInfo.processInfo.isLowPowerModeEnabled)",
            "Thermal state: \(thermalStateDescription(ProcessInfo.processInfo.thermalState))",
            "",
            "Swift app logs",
            "--------------",
            swiftLogs,
        ].joined(separator: "\n")
    }

    private static func bundleValue(_ key: String) -> String {
        Bundle.main.object(forInfoDictionaryKey: key) as? String ?? "unknown"
    }

    private static func deviceModelIdentifier() -> String {
        var systemInfo = utsname()
        uname(&systemInfo)

        let mirror = Mirror(reflecting: systemInfo.machine)
        return mirror.children.reduce(into: "") { identifier, element in
            guard let value = element.value as? Int8, value != 0 else { return }
            identifier.append(String(UnicodeScalar(UInt8(value))))
        }
    }

    private static func thermalStateDescription(_ state: ProcessInfo.ThermalState) -> String {
        switch state {
        case .nominal:
            "nominal"
        case .fair:
            "fair"
        case .serious:
            "serious"
        case .critical:
            "critical"
        @unknown default:
            "unknown"
        }
    }
}
