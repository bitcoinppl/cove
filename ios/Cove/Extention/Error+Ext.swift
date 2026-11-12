//
//  Error+Ext.swift
//  Cove
//

import Foundation

extension Error {
    /// Describes the error for diagnostics logs without its message text
    ///
    /// Swift enum errors, including generated Rust errors, become their case path such as
    /// `TapSignerError.CkTap.BadAuth`, so string payloads that may carry card or user data stay
    /// out of the log. Other errors become their bridged `NSError` domain and code
    var logSummary: String {
        if Mirror(reflecting: self).displayStyle == .enum {
            return "\(type(of: self)).\(Self.casePath(of: self, depth: 0))"
        }

        let nsError = self as NSError
        return "\(nsError.domain) code=\(nsError.code)"
    }

    private static func casePath(of value: Any, depth: Int) -> String {
        let mirror = Mirror(reflecting: value)

        // a payload-less case has no children, and its description is the static case text
        guard let child = mirror.children.first else { return String(describing: value) }

        let caseName = child.label ?? "unknown"
        let payload = Mirror(reflecting: child.value)

        // only recurse into nested enums, never print string or struct payloads
        guard depth < 3, payload.displayStyle == .enum else { return caseName }
        return "\(caseName).\(casePath(of: child.value, depth: depth + 1))"
    }
}
