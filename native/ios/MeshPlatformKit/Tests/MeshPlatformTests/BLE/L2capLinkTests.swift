import Foundation
@testable import MeshPlatform
import XCTest

/// L2capLink over real bound `NSStream` pairs on a real LinkIOThread (the same stream API
/// a CBL2CAPChannel exposes).
final class L2capLinkTests: XCTestCase {
    private var io: LinkIOThread!
    private var sink: RecordingSink!
    private var registry: LinkRegistry!

    override func setUp() {
        super.setUp()
        io = LinkIOThread(name: "test.link-io")
        sink = RecordingSink()
        registry = LinkRegistry(sink: sink)
    }

    override func tearDown() {
        io.stop()
        super.tearDown()
    }

    /// Two links joined by two bound stream pairs (A.out → B.in, B.out → A.in).
    private func pair(bufferSize: Int = 4096, capacity: Int = 4096) -> (L2capLink, L2capLink) {
        var aIn: InputStream?, bOut: OutputStream?, bIn: InputStream?, aOut: OutputStream?
        Stream.getBoundStreams(withBufferSize: bufferSize, inputStream: &aIn, outputStream: &bOut)
        Stream.getBoundStreams(withBufferSize: bufferSize, inputStream: &bIn, outputStream: &aOut)
        let a = L2capLink(id: registry.allocateId(), input: aIn!, output: aOut!, io: io,
                          registry: registry, capacity: capacity)
        let b = L2capLink(id: registry.allocateId(), input: bIn!, output: bOut!, io: io,
                          registry: registry, capacity: capacity)
        for l in [a, b] {
            l.open()
            registry.up(l, info: LinkInfo(id: l.id, kind: .bleL2cap, role: .initiator, maxFrame: 1026,
                                          unit: .stream, peerPlatformHint: .ios))
        }
        return (a, b)
    }

    func testBytesFlowBothWaysUnchanged() {
        let (a, b) = pair()
        let payload = Data((0..<3000).map { UInt8($0 & 0xFF) })
        XCTAssertEqual(a.send(payload.prefix(1026)), .accepted)
        XCTAssertEqual(a.send(payload.dropFirst(1026)), .accepted)
        XCTAssertEqual(b.send(Data([0xAB, 0xCD])), .accepted)
        XCTAssertTrue(waitUntil { self.sink.received(b.id) == payload })
        XCTAssertTrue(waitUntil { self.sink.received(a.id) == Data([0xAB, 0xCD]) })
    }

    func testWouldBlockThenWritableOnce() {
        // Tiny stream buffer and link capacity: the second unit cannot be queued.
        let (a, b) = pair(bufferSize: 64, capacity: 256)
        XCTAssertEqual(a.send(Data(repeating: 1, count: 256)), .accepted)
        var blocked = false
        for _ in 0..<20 where a.send(Data(repeating: 2, count: 200)) == .wouldBlock {
            blocked = true
            break
        }
        XCTAssertTrue(blocked, "a full link refuses instead of buffering without bound")
        // The peer drains its input, so the writer's buffer empties and writable fires.
        XCTAssertTrue(waitUntil { self.sink.writables(a.id) >= 1 })
        XCTAssertTrue(waitUntil { self.sink.received(b.id).count >= 256 })
        XCTAssertEqual(a.pendingBytes, 0)
    }

    func testLocalCloseReportsDownOnceAndPeerSeesEnd() {
        let (a, b) = pair()
        a.close(reason: 3)
        XCTAssertTrue(waitUntil { self.sink.downs(a.id) == [.localClose] })
        XCTAssertTrue(waitUntil { self.sink.downs(b.id) == [.peerClosed] }, "peer stream reports end")
        XCTAssertEqual(a.send(Data([1])), .noLink)
        a.close(reason: 3)
        Thread.sleep(forTimeInterval: 0.05)
        XCTAssertEqual(sink.downs(a.id).count, 1)
    }

    func testIOThreadRunsBlocksInOrder() {
        let order = Recorder<Int>()
        for i in 0..<50 { io.perform { order.add(i) } }
        XCTAssertTrue(waitUntil { order.values.count == 50 })
        XCTAssertEqual(order.values, Array(0..<50))
        let onIO = Recorder<Bool>()
        io.perform { [io] in onIO.add(io!.isCurrent) }
        XCTAssertTrue(waitUntil { onIO.values == [true] })
        XCTAssertFalse(io.isCurrent)
    }

    func testStoppedIOThreadDropsWork() {
        io.stop()
        XCTAssertTrue(io.isStopped)
        let ran = Recorder<Int>()
        io.perform { ran.add(1) }
        Thread.sleep(forTimeInterval: 0.05)
        XCTAssertTrue(ran.values.isEmpty)
    }
}
