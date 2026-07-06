@testable import Cove
import CoveCore
import XCTest

@MainActor
final class NavigationCoordinatorTests: XCTestCase {
    func testAdvanceMarksNavigationUnsettledUntilCurrentGenerationSettles() async {
        let sleeper = NavigationTestSleeper()
        let tracker = TestGenerationTracker()
        let coordinator = NavigationCoordinator(
            routeClient: TestNavigationRouteClient(),
            navigationGenerations: tracker,
            sleep: sleeper.sleep
        )

        let generation = coordinator.advanceNavigationGeneration()

        XCTAssertFalse(coordinator.isNavigationSettled)
        XCTAssertTrue(coordinator.isNavigationGenerationCurrent(generation))

        await sleeper.waitForSleepCount(1)
        sleeper.resumeNext()
        await Task.yield()

        XCTAssertTrue(coordinator.isNavigationSettled)
    }

    func testOlderGenerationCannotSettleAfterNewGenerationAdvances() async {
        let sleeper = NavigationTestSleeper()
        let tracker = TestGenerationTracker()
        let coordinator = NavigationCoordinator(
            routeClient: TestNavigationRouteClient(),
            navigationGenerations: tracker,
            sleep: sleeper.sleep
        )

        let firstGeneration = coordinator.advanceNavigationGeneration()
        await sleeper.waitForSleepCount(1)

        let secondGeneration = coordinator.advanceNavigationGeneration()
        await sleeper.waitForSleepCount(2)

        XCTAssertFalse(coordinator.isNavigationGenerationCurrent(firstGeneration))
        XCTAssertTrue(coordinator.isNavigationGenerationCurrent(secondGeneration))

        sleeper.resumeNext()
        await Task.yield()
        XCTAssertFalse(coordinator.isNavigationSettled)

        sleeper.resumeNext()
        await Task.yield()
        XCTAssertTrue(coordinator.isNavigationSettled)
    }

    func testRapidSidebarNavigationOnlyRunsLatestAction() async {
        let sleeper = NavigationTestSleeper()
        let tracker = TestGenerationTracker()
        let coordinator = NavigationCoordinator(
            routeClient: TestNavigationRouteClient(),
            navigationGenerations: tracker,
            sleep: sleeper.sleep
        )
        var isSidebarVisible = true
        var actions: [String] = []

        coordinator.closeSidebarThenNavigate(isSidebarVisible: &isSidebarVisible) {
            actions.append("first")
        }

        XCTAssertFalse(isSidebarVisible)
        await sleeper.waitForSleepCount(2)

        isSidebarVisible = true
        coordinator.closeSidebarThenNavigate(isSidebarVisible: &isSidebarVisible) {
            actions.append("second")
        }

        XCTAssertFalse(isSidebarVisible)
        await sleeper.waitForSleepCount(4)

        sleeper.resumeFirstSleep(duration: .milliseconds(250))
        await Task.yield()
        XCTAssertTrue(actions.isEmpty)

        sleeper.resumeFirstSleep(duration: .milliseconds(250))
        await Task.yield()
        XCTAssertEqual(actions, ["second"])

        sleeper.resumeAll()
    }
}

private final class TestGenerationTracker: GenerationTrackerProtocol, @unchecked Sendable {
    private var current: UInt64 = 0

    func advance() -> GenerationToken {
        current += 1
        return GenerationToken(value: current)
    }

    func capture() -> GenerationToken {
        GenerationToken(value: current)
    }

    func isCurrent(capturedToken: GenerationToken) -> Bool {
        capturedToken.value == current
    }
}

@MainActor
private final class NavigationTestSleeper {
    private struct PendingSleep {
        let duration: Duration
        let continuation: CheckedContinuation<Void, Error>
    }

    private var continuations: [PendingSleep] = []
    private(set) var sleepDurations: [Duration] = []

    func sleep(_ duration: Duration) async throws {
        sleepDurations.append(duration)
        try await withCheckedThrowingContinuation { continuation in
            continuations.append(PendingSleep(duration: duration, continuation: continuation))
        }
    }

    func resumeNext() {
        let pending = continuations.removeFirst()
        pending.continuation.resume(returning: ())
    }

    func resumeFirstSleep(duration: Duration) {
        guard let index = continuations.firstIndex(where: { $0.duration == duration }) else {
            XCTFail("expected pending sleep for \(duration)")
            return
        }

        let pending = continuations.remove(at: index)
        pending.continuation.resume(returning: ())
    }

    func resumeAll() {
        while !continuations.isEmpty {
            resumeNext()
        }
    }

    func waitForSleepCount(
        _ count: Int,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async {
        for _ in 0 ..< 50 {
            if sleepDurations.count >= count { break }
            await Task.yield()
        }

        XCTAssertEqual(sleepDurations.count, count, file: file, line: line)
    }
}

private final class TestNavigationRouteClient: NavigationRouteClient {
    func dispatch(action _: AppAction) throws {}

    func loadAndResetDefaultRoute(route _: Route) {}

    func resetAfterLoading(to _: [Route]) {}

    func resetDefaultRouteTo(route _: Route) {}

    func resetNestedRoutesTo(defaultRoute _: Route, nestedRoutes _: [Route]) {}
}
