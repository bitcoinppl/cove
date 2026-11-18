import XCTest

/// Creates a new hot wallet through onboarding, reads its recovery words, then verifies every word
/// while bursts of fast taps hit the verify screen: picks cancelled mid-flight, taps during the
/// red dwell and the return, and picks queued behind a running check
///
/// The screen draws the chip from its own state but checks picks in the Rust
/// `WordVerifyStateMachine`, so a tap one side handles and the other never hears about leaves
/// them on different words or states. A word cancelled in flight must never come back into the
/// answer slot, and after each burst the screen must settle on the word it shows and accept that
/// word's correct pick, with the last word reaching the completion screen. A screen stuck on a
/// word, a correct pick judged wrong or a crash fails the test
final class VerifyWordsFullLaunchUITests: XCTestCase {
    private var app: XCUIApplication!

    /// How many word chips word #1 offered, which every later word offers too
    private var chipCount: Int?

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
    }

    func testFastTapsKeepTheScreenAndStateMachineOnTheSameWord() {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])

        let words = createWalletAndReadRecoveryWords()
        openVerifyWords()

        let slot = calibrateSlot(correctWord: words[0])
        var wordNumber = 1

        // each word gets the next burst in turn, so a 12-word wallet runs every burst at least twice
        while wordNumber <= words.count {
            let burst = Burst.allCases[(wordNumber - 1) % Burst.allCases.count]
            let correct = words[wordNumber - 1]
            let options = optionPoints()
            let wrong = options.keys.filter { $0 != correct }.sorted()

            guard let correctPoint = options[correct], wrong.count >= 2 else {
                XCTFail("word #\(wordNumber): expected '\(correct)' and two wrong words among \(options.keys.sorted())")
                return
            }

            let taps = burst.taps.map { target, offset in
                switch target {
                case .correct: (correctPoint, offset)
                case let .wrong(index): (options[wrong[index]] ?? .zero, offset)
                case .slot: (slot, offset)
                }
            }

            let context = "word #\(wordNumber) after \(burst)"
            let cancelled = burst.cancelsFirstWrongPick ? wrong[0] : nil
            guard sendBurst(taps, cancelling: cancelled, on: wordNumber, of: words.count, context: context) else { return }

            wordNumber = settleAndConfirm(wordNumber, after: burst, correct: correct, of: words.count)
        }
    }

    // MARK: - Bursts

    private enum Target {
        case correct
        case wrong(Int)
        case slot
    }

    /// Tap sequences sent as one event, timed against the picked chip, which flies to the answer
    /// slot in about a quarter second, turns green or red there, and flies back when wrong
    private enum Burst: CaseIterable {
        case cancelWrongInFlight
        case cancelWrongThenPickCorrect
        case cancelCorrectThenPickAgain
        case tapsDuringWrongDwell
        case cancelTwiceThenPickCorrect

        /// Taps on the slot every 30 ms from `start` to `end`, so one lands while the flying chip
        /// passes, which takes it only between about 0.12 s and its landing at 0.27 s, later when
        /// the app lags; the ones after it find the slot empty, or a landed chip that only a
        /// flying one can be cancelled from
        private static func slotTaps(from start: TimeInterval, to end: TimeInterval) -> [(Target, TimeInterval)] {
            stride(from: start, through: end + 0.001, by: 0.03).map { (.slot, $0) }
        }

        var taps: [(Target, TimeInterval)] {
            switch self {
            case .cancelWrongInFlight:
                [(.wrong(0), 0)] + Self.slotTaps(from: 0.06, to: 0.33)
            case .cancelWrongThenPickCorrect:
                // a single pick, since a second tap can land on the picked chip as it leaves,
                // which SwiftUI hit tests where it is drawn, and cancel the pick
                [(.wrong(0), 0)] + Self.slotTaps(from: 0.06, to: 0.33) + [(.correct, 0.42)]
            case .cancelCorrectThenPickAgain:
                // the cancelled chip takes taps at its place only once it has flown home, about
                // half a second after the cancel
                [(.correct, 0)] + Self.slotTaps(from: 0.06, to: 0.18) + [(.correct, 0.8)]
            case .tapsDuringWrongDwell:
                [(.wrong(0), 0), (.slot, 0.35), (.slot, 0.45), (.wrong(1), 0.5), (.correct, 0.55)]
            case .cancelTwiceThenPickCorrect:
                [(.wrong(0), 0)] + Self.slotTaps(from: 0.06, to: 0.18)
                    + [(.wrong(1), 0.3)] + Self.slotTaps(from: 0.36, to: 0.48)
                    + [(.correct, 0.6)]
            }
        }

        /// Whether the burst cancels its first wrong pick in flight and picks no other wrong word,
        /// so that word must never come back into the slot
        var cancelsFirstWrongPick: Bool {
            switch self {
            case .cancelWrongInFlight, .cancelWrongThenPickCorrect: true
            case .cancelCorrectThenPickAgain, .tapsDuringWrongDwell, .cancelTwiceThenPickCorrect: false
            }
        }
    }

    /// What the answer slot showed of the word a burst picked and then tapped to cancel
    private enum CancelOutcome {
        /// The word flew home as cancelled and never showed as checked
        case cancelled
        /// The word never showed in the slot, so either it flew home before the slot was first
        /// read or the pick missed, which a single burst can't tell apart. The slot is first read
        /// once the whole burst is sent, by when a cancelled chip has almost always flown home,
        /// so this is the usual outcome of a landed cancel
        case neverSeen
        /// The word showed as checked with no cancel seen first: either every tap on the slot
        /// missed the flight, which an app slow to draw the flying chip makes possible, or the
        /// cancel landed and its check ran on anyway, which a single burst can't tell apart
        case checkedWithoutCancel
        /// The word flew home as cancelled and then showed as checked
        case checkedAfterCancel
    }

    /// Sends a burst, and for one that cancels `cancelled` in flight checks that the word never
    /// shows as checked afterwards, sending the burst again when its cancel may have missed
    ///
    /// Before the fix a cancel only reset the screen: the cancelled check ran on in Rust, landed
    /// about a quarter second after the pick, and put the cancelled word back in the slot as a
    /// wrong guess until about 0.7 s, swallowing any pick made meanwhile. That looks the same as
    /// a cancel that missed, so a burst is sent up to three times and every one of them showing
    /// the word checked fails, which a missed tap alone makes very unlikely. Returns false when
    /// the test failed
    private func sendBurst(
        _ taps: [(CGPoint, TimeInterval)],
        cancelling cancelled: String?,
        on wordNumber: Int,
        of wordCount: Int,
        context: String
    ) -> Bool {
        let attempts = 3
        var trail = ""

        for _ in 0 ..< attempts {
            sendTimedTaps(taps)
            guard let cancelled else { return true }

            let outcome: CancelOutcome
            (outcome, trail) = watchCancelledWord(cancelled)
            print("\(context): '\(cancelled)' \(outcome) (slot: \(trail))")

            switch outcome {
            case .cancelled, .neverSeen:
                return true

            case .checkedAfterCancel:
                XCTFail("\(context): the cancelled word '\(cancelled)' came back into the slot as checked after it flew home, so its check kept running after the cancel (slot: \(trail))")
                return false

            case .checkedWithoutCancel:
                // a correct pick late in the burst may still have moved the screen on, which
                // leaves no word to send the burst at again
                print("\(context): '\(cancelled)' showed as checked, so the burst is sent again (slot: \(trail))")
                waitForIdleChip(context)
                if advanced(past: wordNumber, of: wordCount) { return true }
            }
        }

        XCTFail("\(context): '\(cancelled ?? "")' showed as checked after each of \(attempts) bursts that tapped it in flight, so either its cancel never stops the check or no tap ever reached the flying chip (last slot: \(trail))")
        return false
    }

    /// Watches the answer slot for a second and reports whether `cancelled` showed there as
    /// checked, with the states it went through
    ///
    /// The chip's accessibility value names its state, so a cancelled chip flying home, which
    /// still carries its word, isn't taken for a checked one
    private func watchCancelledWord(_ cancelled: String) -> (CancelOutcome, String) {
        let chip = app.buttons["verifyWords.checkingWord"]
        let start = Date()
        var trail: [String] = []
        var flewHome = false

        while Date().timeIntervalSince(start) < 1 {
            guard let snapshot = try? chip.snapshot() else { continue }

            let state = (snapshot.value as? String) ?? ""
            let entry = "\(snapshot.label)/\(state)"
            if trail.last?.hasSuffix(" \(entry)") != true {
                trail.append(String(format: "%.2fs %@", Date().timeIntervalSince(start), entry))
            }

            guard snapshot.label == cancelled else { continue }

            switch state {
            case "Returning":
                flewHome = true
            case "Correct", "Incorrect":
                return (flewHome ? .checkedAfterCancel : .checkedWithoutCancel, trail.joined(separator: ", "))
            default:
                continue
            }
        }

        return (flewHome ? .cancelled : .neverSeen, trail.joined(separator: ", "))
    }

    /// Waits for the burst's animations to finish, then checks that the screen either moved on
    /// or stayed on its word and still takes a plain correct pick
    ///
    /// The burst's own picks may not move the screen on, since a pick sent a moment after a
    /// cancel doesn't always reach the screen. Returns the word number the screen moved to
    private func settleAndConfirm(_ wordNumber: Int, after burst: Burst, correct: String, of wordCount: Int) -> Int {
        let context = "word #\(wordNumber) after \(burst)"
        waitForIdleChip(context)

        if advanced(past: wordNumber, of: wordCount) { return wordNumber + 1 }

        XCTAssertTrue(
            heading(wordNumber).exists,
            "\(context): the screen left word #\(wordNumber) without reaching the next word or the completion screen"
        )

        let option = app.buttons["verifyWords.option.\(correct)"]
        XCTAssertTrue(option.waitForExistence(timeout: 5), "\(context): '\(correct)' isn't offered")
        XCTAssertTrue(option.isEnabled, "\(context): the screen settled with its word chips disabled")
        option.tap()

        let deadline = Date().addingTimeInterval(8)
        while Date() < deadline {
            if advanced(past: wordNumber, of: wordCount) { return wordNumber + 1 }

            assertRunning(context)
            Thread.sleep(forTimeInterval: 0.25)
        }

        XCTFail("\(context): the correct word '\(correct)' never advanced the screen, so it and the state machine disagree")
        return wordCount + 1
    }

    private func advanced(past wordNumber: Int, of wordCount: Int) -> Bool {
        if wordNumber == wordCount { return app.staticTexts["You're all set!"].exists }

        return heading(wordNumber + 1).exists
    }

    /// Waits until no chip sits in the answer slot for a full second
    private func waitForIdleChip(_ context: String) {
        let chip = app.buttons["verifyWords.checkingWord"]
        let deadline = Date().addingTimeInterval(15)
        var idleSince: Date?

        while Date() < deadline {
            assertRunning(context)

            if chip.exists {
                idleSince = nil
            } else if let since = idleSince, Date().timeIntervalSince(since) >= 1 {
                return
            } else if idleSince == nil {
                idleSince = Date()
            }

            Thread.sleep(forTimeInterval: 0.2)
        }

        XCTFail("\(context): a chip stayed in the answer slot for 15 seconds")
    }

    private func assertRunning(_ context: String) {
        guard app.state != .runningForeground else { return }

        XCTFail("\(context): the app is no longer running (state \(app.state.rawValue)), so it crashed")
    }

    private func heading(_ wordNumber: Int) -> XCUIElement {
        app.staticTexts["What is word #\(wordNumber)?"]
    }

    // MARK: - Points

    /// Reads each word chip's center once the grid has settled
    ///
    /// Right after the screen moves on, the previous word's chips are still fading out, and one
    /// can carry the same word as a new chip at another place, so this waits until the grid
    /// holds as many chips as it did for word #1, no word shows twice, and two reads agree
    private func optionPoints() -> [String: CGPoint] {
        let options = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "verifyWords.option."))
        XCTAssertTrue(options.firstMatch.waitForExistence(timeout: 10), "no word chips on the verify screen")

        let deadline = Date().addingTimeInterval(5)
        var previous: [String: CGPoint]?

        while Date() < deadline {
            let current = readOptionPoints().flatMap { points in
                chipCount == nil || points.count == chipCount ? points : nil
            }

            if let current, current == previous {
                chipCount = current.count
                return current
            }

            previous = current
            Thread.sleep(forTimeInterval: 0.15)
        }

        XCTFail("the word chips never settled")
        return previous ?? [:]
    }

    /// Reads the chips' centers by word, or nil while a word shows on more than one chip
    private func readOptionPoints() -> [String: CGPoint]? {
        guard let snapshot = try? app.snapshot() else { return nil }

        let prefix = "verifyWords.option."
        let options = snapshot.descendants.filter { $0.identifier.hasPrefix(prefix) }
        let points = options.map { option in
            (String(option.identifier.dropFirst(prefix.count)), CGPoint(x: option.frame.midX, y: option.frame.midY))
        }

        let byWord = Dictionary(points, uniquingKeysWith: { first, _ in first })
        return byWord.count == points.count ? byWord : nil
    }

    /// Finds where a picked chip lands by picking a wrong word for word #1 and reading the chip's
    /// frame while it is in the slot, then waits for it to return
    private func calibrateSlot(correctWord: String) -> CGPoint {
        let options = optionPoints()
        guard let wrong = options.keys.filter({ $0 != correctWord }).sorted().first, let point = options[wrong] else {
            XCTFail("word #1 offers no wrong word to calibrate with")
            return .zero
        }

        let chip = app.buttons["verifyWords.checkingWord"]
        sendTimedTaps([(point, 0)])

        // a wrong pick leaves the slot within a second, and waitForExistence first looks about a
        // second after it starts, so this polls
        var frame = CGRect.null
        let deadline = Date().addingTimeInterval(3)
        while Date() < deadline, frame.isNull {
            // one snapshot, since the chip can leave between an exists check and a frame read
            if let snapshot = try? chip.snapshot() { frame = snapshot.frame }
        }

        XCTAssertFalse(frame.isNull, "a picked word never showed in the answer slot")
        waitForIdleChip("word #1 calibration")
        return CGPoint(x: frame.midX, y: frame.midY)
    }

    /// Sends taps at screen points, each starting its offset in seconds after the first, as one
    /// synthesized event
    ///
    /// `XCUIElement.tap()` takes about half a second a tap even with nothing animating, longer
    /// than the chip's flight, so taps that must land mid-flight go through XCTest's event
    /// records, the private API `tap()` is built on; there is no public way to time touches
    private func sendTimedTaps(_ taps: [(CGPoint, TimeInterval)]) {
        guard let record = TimedTouchRecord(name: "verify words burst") else {
            XCTFail("XCTest's event record classes are missing, so timed taps can't be sent")
            return
        }

        for (point, offset) in taps {
            record.addTap(at: point, offset: offset)
        }

        XCTAssertNil(record.send(), "the timed taps weren't delivered")
    }

    // MARK: - Onboarding

    /// Creates a new wallet from onboarding and returns its recovery words in order
    private func createWalletAndReadRecoveryWords() -> [String] {
        let backupTitle = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Back up your wallet")).firstMatch
        let next = app.staticTexts.matching(NSPredicate(format: "label IN %@", ["Terms & Conditions", "Do you already have Bitcoin?"]))

        app.tapButton(startingWith: "Get Started", until: next.firstMatch)
        app.acceptTermsIfNeeded()
        app.tapButton(startingWith: "No, I", until: backupTitle)
        app.tapButton(startingWith: "Show Words", until: app.staticTexts["Your Recovery Words"])

        return readRecoveryWords()
    }

    /// Reads the numbered word cards, pairing each number with the word beside it
    private func readRecoveryWords() -> [String] {
        var words: [Int: String] = [:]

        for _ in 0 ..< 3 {
            guard let snapshot = try? app.snapshot() else { break }

            let texts = snapshot.descendants.filter { $0.elementType == .staticText }
            let numbers = texts.compactMap { text in Int(text.label).map { ($0, text.frame) } }
            let candidates = texts.filter { $0.label.range(of: "^[a-z]+$", options: .regularExpression) != nil }

            for (index, frame) in numbers {
                let word = candidates
                    .filter { abs($0.frame.midY - frame.midY) < 6 && $0.frame.minX >= frame.maxX }
                    .min { $0.frame.minX < $1.frame.minX }

                if let word { words[index] = word.label }
            }

            if [12, 24].contains(words.count) { break }
            app.swipeUp()
        }

        let ordered = (1 ... max(words.count, 1)).compactMap { words[$0] }
        XCTAssertTrue([12, 24].contains(ordered.count), "read \(words.count) recovery words: \(words)")
        return ordered
    }

    private func openVerifyWords() {
        app.tapButton(named: "I Saved These Words", until: app.button(startingWith: "Saved"))
        app.tapButton(named: "Continue")
        app.acceptTermsIfNeeded()

        XCTAssertTrue(heading(1).waitForExistence(timeout: 30), "Continue never opened the verify screen")
    }
}

// MARK: - Timed taps

private extension XCUIElementSnapshot {
    var descendants: [XCUIElementSnapshot] {
        children.flatMap { [$0] + $0.descendants }
    }
}

/// One synthesized event holding several touches, built from XCTest's private
/// `XCSynthesizedEventRecord` and `XCPointerEventPath`, which back `XCUIElement.tap()`
private struct TimedTouchRecord {
    private typealias InitRecord = @convention(c) (Unmanaged<AnyObject>, Selector, NSString, Int) -> Unmanaged<AnyObject>
    private typealias InitPath = @convention(c) (Unmanaged<AnyObject>, Selector, CGPoint, Double) -> Unmanaged<AnyObject>
    private typealias AtOffset = @convention(c) (AnyObject, Selector, Double) -> Void
    private typealias AddPath = @convention(c) (AnyObject, Selector, AnyObject) -> Void
    private typealias Synthesize = @convention(c) (AnyObject, Selector, AnyObject, @escaping @convention(block) (Bool, NSError?) -> Void) -> Void

    private let record: NSObject
    private let pathClass: NSObject.Type

    init?(name: String) {
        guard let recordClass = NSClassFromString("XCSynthesizedEventRecord") as? NSObject.Type,
              let pathClass = NSClassFromString("XCPointerEventPath") as? NSObject.Type,
              let initRecord: InitRecord = Self.instanceImplementation(recordClass, "initWithName:interfaceOrientation:")
        else { return nil }

        // alloc hands back an owned object that init consumes, so both stay unmanaged; 1 is
        // UIInterfaceOrientationPortrait
        let allocated = recordClass.perform(NSSelectorFromString("alloc"))!
        let record = initRecord(allocated, NSSelectorFromString("initWithName:interfaceOrientation:"), name as NSString, 1)
        guard let record = record.takeRetainedValue() as? NSObject else { return nil }

        self.record = record
        self.pathClass = pathClass
    }

    /// Adds a 20 ms touch at `point` that starts `offset` seconds into the event
    func addTap(at point: CGPoint, offset: TimeInterval) {
        guard let initPath: InitPath = Self.instanceImplementation(pathClass, "initForTouchAtPoint:offset:"),
              let addPath: AddPath = Self.implementation(record, "addPointerEventPath:")
        else { return }

        let allocated = pathClass.perform(NSSelectorFromString("alloc"))!
        let path = initPath(allocated, NSSelectorFromString("initForTouchAtPoint:offset:"), point, offset).takeRetainedValue()

        guard let liftUp: AtOffset = Self.implementation(path, "liftUpAtOffset:") else { return }
        liftUp(path, NSSelectorFromString("liftUpAtOffset:"), offset + 0.02)
        addPath(record, NSSelectorFromString("addPointerEventPath:"), path)
    }

    /// Sends the event and waits for XCTest to confirm it was delivered, returning its error
    func send() -> Error? {
        guard let synthesizer = XCUIDevice.shared.value(forKey: "eventSynthesizer") as AnyObject?,
              let synthesize: Synthesize = Self.implementation(synthesizer, "synthesizeEvent:completion:")
        else { return NSError(domain: "TimedTouchRecord", code: 1) }

        var result: Error?
        let delivered = XCTestExpectation(description: "timed taps delivered")
        synthesize(synthesizer, NSSelectorFromString("synthesizeEvent:completion:"), record) { ok, error in
            result = ok ? nil : error ?? NSError(domain: "TimedTouchRecord", code: 2)
            delivered.fulfill()
        }

        if XCTWaiter().wait(for: [delivered], timeout: 30) != .completed {
            return NSError(domain: "TimedTouchRecord", code: 3)
        }

        return result
    }

    private static func implementation<T>(_ object: AnyObject, _ name: String) -> T? {
        let selector = NSSelectorFromString(name)
        guard let object = object as? NSObject, object.responds(to: selector) else { return nil }

        return unsafeBitCast(object.method(for: selector), to: T.self)
    }

    private static func instanceImplementation<T>(_ cls: NSObject.Type, _ name: String) -> T? {
        let selector = NSSelectorFromString(name)
        guard cls.instancesRespond(to: selector) else { return nil }

        return unsafeBitCast(cls.instanceMethod(for: selector), to: T.self)
    }
}
