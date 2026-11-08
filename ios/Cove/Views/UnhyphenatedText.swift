//
//  UnhyphenatedText.swift
//  Cove
//

import SwiftUI

extension Text {
    /// Displays a value exactly as given, never adding a hyphen where a line breaks
    ///
    /// At accessibility text sizes iOS hyphenates a word too long for its line, so a received
    /// website draws as "example.-" over "com" and a key gains hyphens it doesn't have. Marking the
    /// value as having no linguistic content (`zxx`) leaves iOS no hyphenation rules for it: a long
    /// word breaks between characters instead, and spaces still break lines as before
    init(unhyphenated value: String) {
        var text = AttributedString(value)
        text.languageIdentifier = "zxx"
        self.init(text)
    }
}
