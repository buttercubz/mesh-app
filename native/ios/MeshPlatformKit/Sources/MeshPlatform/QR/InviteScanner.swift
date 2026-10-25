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
/// bridge inside an `InviteScannerHost` that adds the close button VisionKit lacks.
/// The first payload that passes the filter finishes the scan.
@MainActor
public final class InviteScanner: NSObject, DataScannerViewControllerDelegate {
    private let filter: InvitePayloadFilter
    var completion: ((InviteScanResult) -> Void)?
    private weak var scanner: DataScannerViewController?
    private weak var host: UIViewController?

    public init(filter: InvitePayloadFilter = .invite()) {
        self.filter = filter
    }

    public static var isSupported: Bool { DataScannerViewController.isSupported }
    public static var isAvailable: Bool { DataScannerViewController.isAvailable }

    /// Presents the scanner over `presenter`; `completion` runs once. `closeLabel` is the
    /// accessibility label of the close button (the bridge passes it localized).
    public func present(from presenter: UIViewController, closeLabel: String = "Cancel",
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
        scanner = vc
        let host = InviteScannerHost(content: vc, closeLabel: closeLabel) { [weak self] in self?.cancel() }
        self.host = host
        presenter.present(host, animated: true) {
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
        scanner?.stopScanning()
        if let h = host, h.presentingViewController != nil {
            h.dismiss(animated: true)
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

/// Full-screen container for the scanner: the scanner fills it as a child view
/// controller and the close button sits above it as a sibling. VisionKit's own views
/// (`overlayContainerView` included) swallow touches, so a button placed inside the
/// scanner never fires (field report 2026-10-25: the scanner could not be closed).
@MainActor
final class InviteScannerHost: UIViewController {
    private let content: UIViewController
    private let closeLabel: String
    private let onClose: () -> Void
    private(set) var closeButton: UIButton?

    init(content: UIViewController, closeLabel: String, onClose: @escaping () -> Void) {
        self.content = content
        self.closeLabel = closeLabel
        self.onClose = onClose
        super.init(nibName: nil, bundle: nil)
        modalPresentationStyle = .fullScreen
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        addChild(content)
        content.view.frame = view.bounds
        content.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(content.view)
        content.didMove(toParent: self)

        var config = UIButton.Configuration.filled()
        config.image = UIImage(systemName: "xmark")
        config.cornerStyle = .capsule
        config.baseBackgroundColor = UIColor.black.withAlphaComponent(0.6)
        config.baseForegroundColor = .white
        let button = UIButton(configuration: config, primaryAction: UIAction { [weak self] _ in self?.onClose() })
        button.accessibilityLabel = closeLabel
        button.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(button)
        NSLayoutConstraint.activate([
            button.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            button.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),
            button.widthAnchor.constraint(equalToConstant: 44),
            button.heightAnchor.constraint(equalToConstant: 44),
        ])
        closeButton = button
    }

    /// VoiceOver's two-finger scrub closes the scanner too.
    override func accessibilityPerformEscape() -> Bool {
        onClose()
        return true
    }
}
