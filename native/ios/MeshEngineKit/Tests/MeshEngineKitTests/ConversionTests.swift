@preconcurrency import MeshCoreBindings
import Foundation
@testable import MeshEngineKit
import MeshPlatform
import XCTest

final class ConversionTests: XCTestCase {
    private let groupTag = Data([0xA1, 0xB2, 0xC3, 0xD4])
    private let token = Data([0x5A, 0x6B, 0x7C, 0x8D])

    // MARK: Advertising

    func testAdvPayloadFromServiceDataKeepsEveryToken() throws {
        let sd = AdvBuilder.serviceData(android: false, lab: true, groupTag: [UInt8](groupTag),
                                        nodeToken: [UInt8](token))
        let p = FfiAdvPayload(serviceData: sd, localName: "IGNORE")
        let adv = try XCTUnwrap(AdvConfig(p))
        XCTAssertEqual(adv, AdvConfig(groupTag: groupTag, nodeToken: token, lab: true))
        // The iPhone local name the driver advertises equals the core's.
        XCTAssertEqual(AdvBuilder.iosLocalName(groupTag: [UInt8](adv.groupTag), nodeToken: [UInt8](adv.nodeToken)),
                       AdvBuilder.iosLocalName(groupTag: [UInt8](groupTag), nodeToken: [UInt8](token)))
    }

    func testAdvPayloadFallsBackToTheLocalName() throws {
        let name = AdvBuilder.iosLocalName(groupTag: [UInt8](groupTag), nodeToken: [UInt8](token))
        let adv = try XCTUnwrap(AdvConfig(FfiAdvPayload(serviceData: Data(), localName: name)))
        XCTAssertEqual(AdvBuilder.iosLocalName(groupTag: [UInt8](adv.groupTag), nodeToken: [UInt8](adv.nodeToken)),
                       name)
        XCTAssertNil(AdvConfig(FfiAdvPayload(serviceData: Data([1, 2]), localName: "x")))
    }

    func testDiscoveryTokensAreReEncodedForTheCore() {
        // iPhone foreground: local name round trip.
        let name = AdvBuilder.iosLocalName(groupTag: [UInt8](groupTag), nodeToken: [UInt8](token))
        let info = AdvBuilder.parseLocalName(name)!
        let ios = Discovery(discId: 7, transport: .ble, rssi: -60,
                            tokens: AdvTokens(groupTag: Data(info.groupTagPrefix), tokenPrefix14: info.tokenPrefix14),
                            platformHint: .ios)
        let f = FfiDiscovery(ios)
        XCTAssertEqual(f.discId, 7)
        XCTAssertEqual(f.rssi, -60)
        XCTAssertEqual(f.adv, .localName(name: name))
        XCTAssertEqual(f.platformHint, .ios)
        // Android scan response: the 9-byte service data body, byte for byte.
        let body = AdvBuilder.serviceData(android: true, lab: false, groupTag: [UInt8](groupTag),
                                          nodeToken: [UInt8](token))
        let parsed = AdvBuilder.parseServiceData(body)!
        let android = FfiDiscovery(Discovery(discId: 8, transport: .ble, rssi: nil, tokens: parsed,
                                             platformHint: .android))
        XCTAssertEqual(android.adv, .serviceData(bytes: body))
        XCTAssertEqual(android.platformHint, .android)
        // Background iPhone: UUID only, unknown platform → nil hint.
        let bg = FfiDiscovery(Discovery(discId: 9, transport: .ble, rssi: -90, tokens: nil, platformHint: .unknown))
        XCTAssertEqual(bg.adv, .uuidOnly)
        XCTAssertNil(bg.platformHint)
    }

    func testTokenBytesInvertThePrefix() {
        for p: UInt16 in [0, 1, 0x3FFF, 0x2A5C, 0x1234 & 0x3FFF] {
            XCTAssertEqual(AdvBuilder.tokenPrefix14(FfiAdvData.tokenBytes(prefix14: p)), p)
        }
    }

    // MARK: Radio commands

    func testRadioCommandsMapOneToOne() {
        XCTAssertEqual(RadioCommand(.connect(discId: 3)), .connect(discId: 3))
        XCTAssertEqual(RadioCommand(.setHello(bytes: Data([1, 2]))), .setHello(Data([1, 2])))
        XCTAssertEqual(RadioCommand(.openLink(discId: 4, via: .l2cap(psm: 0x81))), .openLink(discId: 4, via: .l2cap(psm: 0x81)))
        XCTAssertEqual(RadioCommand(.openLink(discId: 4, via: .gatt)), .openLink(discId: 4, via: .gatt))
        XCTAssertEqual(RadioCommand(.abort(discId: 5, ignoreS: 30)), .abort(discId: 5, ignoreS: 30))
        XCTAssertEqual(RadioCommand(.close(link: 6, reason: nil)), .close(link: 6, reason: 0))
        XCTAssertEqual(RadioCommand(.close(link: 6, reason: 3)), .close(link: 6, reason: 3))
        XCTAssertEqual(RadioCommand(.setLinkParams(link: 2, priority: .high)), .setLinkParams(link: 2, priority: .high))
        XCTAssertEqual(RadioCommand(.armReconnect(peerRef: 9, untilS: 900)), .armReconnect(discId: 9, untilS: 900))
        XCTAssertEqual(RadioCommand(.awareUpgrade(peer: 12, op: 1)), .awareUpgrade(peer: 12, op: 1))
        XCTAssertEqual(RadioCommand(.setDiscovery(mode: .off, adv: nil, scan: .lowPower)), .setDiscovery(mode: .off, adv: nil))
        let sd = AdvBuilder.serviceData(android: false, lab: false, groupTag: [UInt8](groupTag), nodeToken: [UInt8](token))
        XCTAssertEqual(RadioCommand(.setDiscovery(mode: .forming, adv: FfiAdvPayload(serviceData: sd, localName: ""), scan: .lowLatency)),
                       .setDiscovery(mode: .forming, adv: AdvConfig(groupTag: groupTag, nodeToken: token, lab: false)))
    }

    // MARK: Links

    func testLinkInfoAndReasons() {
        let i = LinkInfo(id: 3, kind: .bleL2cap, role: .initiator, maxFrame: 1026, unit: .stream,
                         peerPlatformHint: .android, discId: 101)
        XCTAssertEqual(FfiLinkInfo(i), FfiLinkInfo(kind: .bleL2cap, role: .initiator, maxFrame: 1026, unit: .stream,
                                                   peerPlatformHint: .android, attMtu: nil, discId: 101))
        let g = LinkInfo(id: 4, kind: .bleGatt, role: .responder, maxFrame: 182, unit: .message, peerPlatformHint: .unknown)
        XCTAssertEqual(FfiLinkInfo(g).unit, .message)
        XCTAssertNil(FfiLinkInfo(g).peerPlatformHint)
        XCTAssertEqual(FfiLinkDownReason(.limitReached), .limitReached)
        XCTAssertEqual(FfiLinkDownReason(.radioOff), .radioOff)
        XCTAssertEqual(FfiLinkDownReason(.error(-7)), .error(code: -7))
        XCTAssertEqual(FfiSendOutcome(.wouldBlock), .wouldBlock)
    }

    // MARK: Audio

    func testAudioRouteAndIoStates() {
        let r = AudioRouteInfo(kind: .hfpNarrowband, inputRate: 8000, output: .bluetoothHfp, headsetPresent: true)
        XCTAssertEqual(FfiAudioRoute(r), FfiAudioRoute(kind: .hfpNarrowband, inputRate: 8000, output: .hfpNarrowband,
                                                       headsetPresent: true))
        XCTAssertEqual(FfiAudioRoute(.none).kind, .none)
        XCTAssertEqual(FfiAudioIoState(.interrupted(.otherSession), onCall: true), .interrupted(reason: .call))
        XCTAssertEqual(FfiAudioIoState(.interrupted(.otherSession), onCall: false), .interrupted(reason: .otherApp))
        XCTAssertEqual(FfiAudioIoState(.interrupted(.routeDisconnected), onCall: false), .interrupted(reason: .routeChange))
        XCTAssertEqual(FfiAudioIoState(.captureLost, onCall: false), .captureLost)
        XCTAssertEqual(FfiAudioIoState(.restored, onCall: false), .restored)
        XCTAssertEqual(FfiRadioAvail(.unauthorized), .unauthorized)
        XCTAssertEqual(FfiThermal(.serious), .serious)
        XCTAssertEqual(VehicleProfile(apiValue: "car"), .car)
        XCTAssertEqual(VehicleProfile(apiValue: "moto"), .motorcycle)
        XCTAssertNil(VehicleProfile(apiValue: "bike"))
    }
}
