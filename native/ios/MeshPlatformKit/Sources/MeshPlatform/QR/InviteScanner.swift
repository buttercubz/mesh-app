import Foundation
import UIKit
import VisionKit

/// Accepts or rejects scanned QR payloads. The default accepts the app's invite links
/// (`convlinkvoice://join?v=1#<invite>`, DESIGN §8.3); the payload is passed verbatim to
/// `parseInvite` by the bridge, so validation of the content itself stays in the core.
public struct InvitePayloadFilter: Sendable {
    public var accept: @Sendable (String) -> Bool

    public init(accept: @escaping @Sendable (String) -> Bool) {
        self.accept = accept
    }

    public static func invite(scheme: String = "convlinkvoice") -> InvitePayloadFilter {
        let prefix = scheme.lowercased() + "://join"
        return InvitePayloadFilter { $0.lowercased().hasPrefix(prefix) }
    }

    public static let any = InvitePayloadFilter { !$0.isEmpty }
}

/// Outcome of a scan session.
public enum InviteScanResult: Sendable, Equatable {
    case payload(String)
    case cancelled
    /// The device has no camera / DataScanner support (simulator, old device).
    case unsupported
    /// Camera permission denied or restricted.
    case unavailable
}

/// QR invite scanner (`scanInviteQr`, DESIGN §10.1.1): VisionKit
/// `DataScannerViewController` with `.barcode([.qr])`, presented full screen by the
/// bridge. The first payload that passes the filter finishes the scan.
@MainActor
public final class InviteScanner: NSObject, DataScannerViewControllerDelegate {
    private let filter: InvitePayloadFilter
    var completion: ((InviteScanResult) -> Void)?
    private weak var scanner: DataScannerViewController?

    public init(filter: InvitePayloadFilter = .invite()) {
        self.filter = filter
    }

    public static var isSupported: Bool { DataScannerViewController.isSupported }
    public static var isAvailable: Bool { DataScannerViewController.isAvailable }

    /// Presents the scanner over `presenter`; `completion` runs once.
    public func present(from presenter: UIViewController,
                        completion: @escaping (InviteScanResult) -> Void) {
        guard Self.isSupported else { return completion(.unsupported) }
        guard Self.isAvailable else { return completion(.unavailable) }
        self.completion = completion
        let vc = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])],
            qualityLevel: .balanced,
            recognizesMultipleItems: false,
            isHighFrameRateTrackingEnabled: false,
            isPinchToZoomEnabled: true,
            isGuidanceEnabled: true,
            isHighlightingEnabled: true)
        vc.delegate = self
        vc.modalPresentationStyle = .fullScreen
        scanner = vc
        presenter.present(vc, animated: true) {
            do {
                try vc.startScanning()
            } catch {
                self.finish(.unavailable)
            }
        }
    }

    /// Cancels an open scan (bridge timeout, app backgrounded).
    public func cancel() {
        finish(.cancelled)
    }

    /// Filters recognized items (also the unit-test entry point).
    public func consider(_ payloads: [String]) {
        guard let p = payloads.first(where: { filter.accept($0) }) else { return }
        finish(.payload(p))
    }

    private func finish(_ r: InviteScanResult) {
        guard let done = completion else { return }
        completion = nil
        if let s = scanner {
            s.stopScanning()
            s.dismiss(animated: true)
        }
        done(r)
    }

    public func dataScanner(_ dataScanner: DataScannerViewController, didAdd addedItems: [RecognizedItem],
                            allItems: [RecognizedItem]) {
        consider(addedItems.compactMap(Self.payload))
    }

    public func dataScanner(_ dataScanner: DataScannerViewController, didTapOn item: RecognizedItem) {
        consider([item].compactMap(Self.payload))
    }

    public func dataScanner(_ dataScanner: DataScannerViewController,
                            becameUnavailableWithError error: DataScannerViewController.ScanningUnavailable) {
        finish(.unavailable)
    }

    static func payload(_ item: RecognizedItem) -> String? {
        if case .barcode(let b) = item { return b.payloadStringValue }
        return nil
    }
}
