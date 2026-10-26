import Foundation
@testable import MeshPlatform
import UIKit
import XCTest

final class QRRendererTests: XCTestCase {
    /// ≈ 170-character invite link (DESIGN §8.3: QR version ≈ 10, ECC M).
    private let invite = "convlinkvoice://join?v=1#" + String(repeating: "AbCdEfGh-_12", count: 12)

    func testRendersADecodablePng() throws {
        let png = try QRRenderer.png(invite, size: 600)
        let img = try XCTUnwrap(UIImage(data: png))
        XCTAssertEqual(img.size.width, img.size.height)
        XCTAssertGreaterThanOrEqual(img.size.width * img.scale, 400)
        XCTAssertLessThanOrEqual(img.size.width * img.scale, 600)
        XCTAssertEqual(QRRenderer.decode(png: png), [invite], "round trip through a QR detector")
    }

    func testDataUrlAndErrors() throws {
        let url = try QRRenderer.dataURL("hello", size: 200)
        XCTAssertTrue(url.hasPrefix("data:image/png;base64,"))
        let b64 = String(url.dropFirst("data:image/png;base64,".count))
        XCTAssertEqual(QRRenderer.decode(png: try XCTUnwrap(Data(base64Encoded: b64))), ["hello"])
        XCTAssertThrowsError(try QRRenderer.png("")) { XCTAssertEqual($0 as? QRRenderer.RenderError, .empty) }
    }
}

@MainActor
final class InviteScannerTests: XCTestCase {
    func testDefaultFilterAcceptsOnlyInviteLinks() {
        let f = InvitePayloadFilter.invite()
        XCTAssertTrue(f.accept("convlinkvoice://join?v=1#abc"))
        XCTAssertTrue(f.accept("CONVLINKVOICE://JOIN?v=1#abc"))
        XCTAssertFalse(f.accept("https://example.com"))
        XCTAssertFalse(f.accept("convlinkvoice://settings"))
        XCTAssertTrue(InvitePayloadFilter.any.accept("x"))
        XCTAssertFalse(InvitePayloadFilter.any.accept(""))
    }

    func testFirstAcceptedPayloadFinishesOnce() {
        let s = InviteScanner()
        // Results go to a Recorder, not a captured `var`: Swift 6.3.3 (CI's Xcode 26.6)
        // crashes in MandatoryAllocBoxToStack on this file's captured mutable locals.
        let results = Recorder<InviteScanResult>()
        s.completion = { results.add($0) }
        s.consider(["https://menu.example", "convlinkvoice://join?v=1#first", "convlinkvoice://join?v=1#second"])
        s.consider(["convlinkvoice://join?v=1#late"])
        s.cancel()
        XCTAssertEqual(results.values, [.payload("convlinkvoice://join?v=1#first")])
    }

    func testSimulatorHasNoScannerSoPresentReportsUnsupported() {
        // DataScannerViewController needs a camera and the Neural Engine.
        let s = InviteScanner()
        let result = Recorder<InviteScanResult>()
        s.present(from: UIViewController()) { result.add($0) }
        if InviteScanner.isSupported {
            XCTAssertNotEqual(result.values.first, .unsupported)
        } else {
            XCTAssertEqual(result.values, [.unsupported])
        }
    }
}

@MainActor
final class InviteScannerHostTests: XCTestCase {
    /// The close button must sit above the scanner as a sibling, never inside it: views
    /// inside VisionKit's scanner do not receive taps (field report 2026-10-25).
    func testCloseButtonIsOnTopOfTheScannerAndCloses() throws {
        let content = UIViewController()
        let closes = Recorder<Int>()
        let host = InviteScannerHost(content: content, closeLabel: "Cancelar") { closes.add(1) }
        host.loadViewIfNeeded()
        let button = try XCTUnwrap(host.closeButton)
        XCTAssertTrue(button.superview === host.view)
        XCTAssertTrue(host.view.subviews.last === button, "button is the topmost view")
        XCTAssertFalse(button.isDescendant(of: content.view))
        XCTAssertTrue(content.parent === host)
        XCTAssertEqual(button.accessibilityLabel, "Cancelar")
        XCTAssertEqual(host.modalPresentationStyle, .fullScreen)
        button.sendActions(for: .primaryActionTriggered)
        XCTAssertEqual(closes.values.count, 1)
        XCTAssertTrue(host.accessibilityPerformEscape())
        XCTAssertEqual(closes.values.count, 2)
    }

    /// A tap at the button's centre hit-tests to the button, not to the scanner's views.
    func testTapOnCloseButtonHitsTheButton() throws {
        let content = UIViewController()
        let host = InviteScannerHost(content: content, closeLabel: "Cancel") {}
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        window.rootViewController = host
        window.makeKeyAndVisible()
        host.view.layoutIfNeeded()
        let button = try XCTUnwrap(host.closeButton)
        let centre = button.convert(CGPoint(x: button.bounds.midX, y: button.bounds.midY), to: host.view)
        let hit = try XCTUnwrap(host.view.hitTest(centre, with: nil))
        XCTAssertTrue(hit === button || hit.isDescendant(of: button))
    }
}

