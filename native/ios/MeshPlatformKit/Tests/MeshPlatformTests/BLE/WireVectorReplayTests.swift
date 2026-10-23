import CoreBluetooth
import Foundation
@testable import MeshPlatform
import XCTest

/// Replays `test-vectors/wire/hello.json` and `adv.json` (WP-14 acceptance: the Swift
/// adv/HELLO builders reproduce the Rust golden vectors byte for byte).
final class WireVectorReplayTests: XCTestCase {
    func testHelloVectors() throws {
        let vectors = try VectorFiles.load("hello.json")
        XCTAssertGreaterThanOrEqual(vectors.count, 4)
        for v in vectors {
            let i = v.inputs
            let flags = Hello.Flags(rawValue: UInt8(try XCTUnwrap(i.int("flags"))))
            let psm = UInt16(try XCTUnwrap(i.int("psm")))
            let token = bytes(hex: try XCTUnwrap(i.str("node_token")))
            let hello: Hello
            if let labTag = i.int("lab_tag") {
                if let code = i.int("lab_code") {
                    XCTAssertEqual(Int(Hello.labTag(labCode: UInt16(code))), labTag, "\(v.name): lab_tag derivation")
                }
                hello = Hello.lab(flags: flags, psm: psm, labTag: UInt16(labTag), nodeToken: token)
            } else {
                let epoch = UInt32(try XCTUnwrap(i.int("epoch")))
                hello = Hello.group(flags: flags, psm: psm,
                                    groupTag: bytes(hex: try XCTUnwrap(i.str("group_tag"))),
                                    nodeToken: token, epoch: epoch)
                XCTAssertEqual(Int(hello.epochLo), i.int("epoch_lo"), v.name)
            }
            let encoded = try hello.encode()
            XCTAssertEqual(hex(encoded), v.expected, v.name)
            XCTAssertEqual(try Hello.decode(encoded), hello, "\(v.name): decode round trip")
        }
    }

    func testHelloDecodeRejectsBadInput() throws {
        let good = Data(bytes(hex: "c10101048100a1b2c3d41f2e3d4c0700"))
        XCTAssertThrowsError(try Hello.decode(good.prefix(15)))
        XCTAssertThrowsError(try Hello.decode(good + Data([0])))
        var bad = good
        bad[0] = 0xC2
        XCTAssertThrowsError(try Hello.decode(bad))
        bad = good
        bad[1] = 3
        bad[2] = 2
        XCTAssertThrowsError(try Hello.decode(bad))
        XCTAssertThrowsError(try Hello(flags: [], psm: 0, groupTag: [1], nodeToken: [1, 2, 3, 4], epochLo: 0).encode())
    }

    func testAdvVectors() throws {
        let vectors = try VectorFiles.load("adv.json")
        let svc = BleConstants.design.meshService
        var checked = Set<String>()
        for v in vectors {
            let i = v.inputs
            switch v.name {
            case "uuid/mesh_svc":
                XCTAssertEqual(hex(svc.canonicalBytes), v.expected)
            case "uuid/hello":
                XCTAssertEqual(hex(BleConstants.design.hello.canonicalBytes), v.expected)
            case "uuid/rx":
                XCTAssertEqual(hex(BleConstants.design.rx.canonicalBytes), v.expected)
            case "uuid/tx":
                XCTAssertEqual(hex(BleConstants.design.tx.canonicalBytes), v.expected)
            case "uuid/mesh_svc-le":
                XCTAssertEqual(hex(svc.littleEndianBytes), v.expected)
            case "android/adv-data-21":
                XCTAssertEqual(hex(AdvBuilder.androidAdvData(service: svc)), v.expected)
            case let n where n.hasPrefix("android/scan-response"):
                let flags = try XCTUnwrap(i.int("flags"))
                let sd = AdvBuilder.serviceData(android: flags & 1 != 0, lab: flags & 2 != 0,
                                                groupTag: bytes(hex: try XCTUnwrap(i.str("group_tag"))),
                                                nodeToken: bytes(hex: try XCTUnwrap(i.str("node_token"))))
                XCTAssertEqual(hex(sd), i.str("service_data"), v.name)
                XCTAssertEqual(hex(AdvBuilder.scanResponse(service: svc, serviceData: sd)), v.expected, v.name)
                let parsed = try XCTUnwrap(AdvBuilder.parseServiceData(sd))
                XCTAssertEqual(parsed.android, flags & 1 != 0)
                XCTAssertEqual(parsed.lab, flags & 2 != 0)
            case let n where n.hasPrefix("ios/local-name"):
                let tag = bytes(hex: try XCTUnwrap(i.str("group_tag")))
                let token = bytes(hex: try XCTUnwrap(i.str("node_token")))
                let name = AdvBuilder.iosLocalName(groupTag: tag, nodeToken: token)
                XCTAssertEqual(name, i.str("local_name"), v.name)
                XCTAssertEqual(Int(AdvBuilder.tokenPrefix14(token)), i.int("token_prefix14"), v.name)
                XCTAssertEqual(hex(AdvBuilder.iosLocalNameAd(groupTag: tag, nodeToken: token)), v.expected, v.name)
                let info = try XCTUnwrap(AdvBuilder.parseLocalName(name))
                XCTAssertEqual(Int(info.tokenPrefix14), i.int("token_prefix14"))
                XCTAssertTrue(info.matches(groupTag: tag))
            default:
                XCTFail("unhandled adv vector \(v.name): extend the replay")
            }
            checked.insert(v.name)
        }
        XCTAssertEqual(checked.count, vectors.count)
    }

    func testLocalNameIsSixBase32CharsAndRejectsOthers() {
        XCTAssertNil(AdvBuilder.parseLocalName("iPhone"))
        XCTAssertNil(AdvBuilder.parseLocalName("ABCDE"))
        XCTAssertNil(AdvBuilder.parseLocalName("ABCDE1"), "1 is not in the RFC 4648 alphabet")
        XCTAssertNotNil(AdvBuilder.parseLocalName("AAAAAA"))
    }

    func testIosAdvertisementDictionary() throws {
        let adv = AdvConfig(groupTag: Data([0xA1, 0xB2, 0xC3, 0xD4]), nodeToken: Data([0x1F, 0x2E, 0x3D, 0x4C]),
                            lab: false)
        let d = AdvBuilder.iosAdvertisement(service: BleConstants.design.meshService, adv: adv)
        XCTAssertEqual(d[CBAdvertisementDataLocalNameKey] as? String, "UGZB6L")
        XCTAssertEqual(d[CBAdvertisementDataServiceUUIDsKey] as? [CBUUID], [BleConstants.design.meshService])
        XCTAssertEqual(d.count, 2, "no mesh data in advertisements (§9.2)")
    }
}
