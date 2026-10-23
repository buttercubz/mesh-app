import CoreImage
import CoreImage.CIFilterBuiltins
import Foundation
import UIKit

/// Native QR rendering for `getInvite` (`qrPng`, DESIGN §8.1): the invite text goes from
/// the core straight into this image, so it never enters JS state.
public enum QRRenderer {
    public enum Correction: String, Sendable {
        case low = "L", medium = "M", quartile = "Q", high = "H"
    }

    public enum RenderError: Error, Equatable, Sendable {
        case empty
        case generator
        case encode
    }

    /// Renders `payload` as a PNG of about `size` px (integer module scaling, 4-module
    /// quiet zone, black on white).
    public static func png(_ payload: String, size: Int = 600, correction: Correction = .medium) throws(RenderError) -> Data {
        guard !payload.isEmpty else { throw .empty }
        let f = CIFilter.qrCodeGenerator()
        f.message = Data(payload.utf8)
        f.correctionLevel = correction.rawValue
        guard let raw = f.outputImage else { throw .generator }
        let quiet: CGFloat = 4
        let modules = raw.extent.width
        let scale = max(1, (CGFloat(size) / (modules + 2 * quiet)).rounded(.down))
        let qr = raw.transformed(by: CGAffineTransform(scaleX: scale, y: scale))
        let side = (modules + 2 * quiet) * scale
        let canvas = CIImage(color: .white).cropped(to: CGRect(x: 0, y: 0, width: side, height: side))
        let placed = qr.transformed(by: CGAffineTransform(translationX: quiet * scale, y: quiet * scale))
            .composited(over: canvas)
        let ctx = CIContext(options: [.useSoftwareRenderer: false])
        guard let cg = ctx.createCGImage(placed, from: placed.extent) else { throw .encode }
        guard let data = UIImage(cgImage: cg).pngData() else { throw .encode }
        return data
    }

    /// `data:image/png;base64,…` for the plugin result.
    public static func dataURL(_ payload: String, size: Int = 600) throws(RenderError) -> String {
        "data:image/png;base64," + (try png(payload, size: size)).base64EncodedString()
    }

    /// Decodes QR payloads found in a PNG (tests and self-checks).
    public static func decode(png: Data) -> [String] {
        guard let img = CIImage(data: png),
              let det = CIDetector(ofType: CIDetectorTypeQRCode, context: nil,
                                   options: [CIDetectorAccuracy: CIDetectorAccuracyHigh]) else { return [] }
        return det.features(in: img).compactMap { ($0 as? CIQRCodeFeature)?.messageString }
    }
}
