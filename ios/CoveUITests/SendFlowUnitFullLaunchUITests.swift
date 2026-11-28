import XCTest

/// Sends in the unit picked on the wallet screen against cove-fake-node, then checks what the app
/// broadcast
///
/// Needs the fake node on the ports `COVE_UI_TEST_PORT_BASE` names, which `xtask ios-ui`
/// (`just ios-ui-background`) starts for every run, and a freshly erased simulator, which xtask
/// also provides
final class SendFlowUnitFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl
    private static let feeUrl = "\(esploraUrl)/api/v1/fees/recommended"

    private var app: XCUIApplication!
    private var node: FakeNodeControl!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        node = FakeNodeControl()

        // fee rates come from the fake node on every launch, so sends don't depend on mempool.space
        app.launchEnvironment["COVE_FEE_URL"] = Self.feeUrl
    }

    func testSatsPickedOnWalletScreenIsTheSendAmountUnit() throws {
        try app.launchFundedWallet(node: node)

        // 1: show the balance in sats, then send an amount typed in sats
        app.useSatsOnWalletScreen()
        app.startSend(to: XCUIApplication.externalAddress)
        waitForLabel(app.sendUnitSelector, equalTo: "sats", timeout: 10)
        app.typeSendAmount("50000")
        app.tapNext()
        app.swipeToSend()

        let satsSend = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(satsSend.sentToExternal, 50000, "50000 typed in sats must send 50,000 sats")

        // 2: switching the send screen back to btc converts what was typed and sends in btc
        app.startSend(to: XCUIApplication.externalAddress)
        waitForLabel(app.sendUnitSelector, equalTo: "sats", timeout: 10)
        app.typeSendAmount("20000")
        app.selectSendUnit("btc")
        app.waitForSendAmountText("0.0002", "20000 sats didn't convert to 0.0002 btc")
        app.tapNext()
        app.swipeToSend()

        let btcSend = try node.waitForBroadcasts(count: 2)[1]
        XCTAssertEqual(btcSend.sentToExternal, 20000, "0.0002 btc must send 20,000 sats")
    }

    /// A Send screen opened in btc, then left, keeps no hold on the unit when the wallet switches
    /// to sats and Send opens again straight away
    func testSatsPickedRightAfterABtcSendIsTheSendAmountUnit() throws {
        try app.launchFundedWallet(node: node)

        // a send session in btc that the user backs out of
        app.startSend(to: XCUIApplication.externalAddress)
        waitForLabel(app.sendUnitSelector, equalTo: "btc", timeout: 10)
        app.goBack()

        // switch to sats and open Send without waiting for the wallet screen to settle
        app.useSatsOnWalletScreen(waitForBalance: false)
        app.startSend(to: XCUIApplication.externalAddress)
        waitForLabel(app.sendUnitSelector, equalTo: "sats", timeout: 10)
        app.typeSendAmount("50000")
        app.tapNext()
        app.swipeToSend()

        let send = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(send.sentToExternal, 50000, "50000 typed in sats must send 50,000 sats")
    }
}
