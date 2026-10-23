import Foundation
import Synchronization

/// An L2CAP CoC link over the channel's `NSStream` pair (DESIGN §9.1, §9.5).
///
/// Bytes are opaque: the core frames (`len u16 ‖ frame`) and deframes, so a `send` unit is
/// written as is and inbound chunks are delivered at any boundary. `send` never blocks: it
/// queues into a bounded buffer that the IO thread drains on `hasSpaceAvailable`; when the
/// buffer cannot take a unit it returns `.wouldBlock` and reports `linkWritable` once the
/// buffer has drained.
public final class L2capLink: NSObject, MeshLink, StreamDelegate, @unchecked Sendable {
    public let id: UInt32
    public let kind: LinkKind = .bleL2cap

    private struct Out {
        var pending = Data()
        var wantWritable = false
        var closed = false
    }

    // Streams are touched only on `io`; `out` is shared with the engine thread (Mutex);
    // `channel` only retains the CBL2CAPChannel that owns the streams.
    private let input: InputStream
    private let output: OutputStream
    private let io: LinkIOThread
    private let registry: LinkRegistry
    private let channel: AnyObject?
    private let out = Mutex(Out())
    private let capacity: Int
    private var readBuffer = [UInt8](repeating: 0, count: 4096)

    /// - Parameters:
    ///   - capacity: bytes the link may hold before `send` returns `.wouldBlock`
    ///     (≥ 2 frames of `maxFrame`).
    ///   - channel: the CBL2CAPChannel (retained for the link's lifetime), nil in tests.
    public init(id: UInt32, input: InputStream, output: OutputStream, io: LinkIOThread,
                registry: LinkRegistry, capacity: Int = 4096, channel: AnyObject? = nil) {
        self.id = id
        self.input = input
        self.output = output
        self.io = io
        self.registry = registry
        self.capacity = capacity
        self.channel = channel
        super.init()
    }

    /// Opens the streams on the IO thread; call once, before `registry.up`.
    public func open() {
        io.perform { [self] in
            input.delegate = self
            output.delegate = self
            io.schedule([input, output])
            input.open()
            output.open()
        }
    }

    public func send(_ bytes: Data) -> SendOutcome {
        let outcome: SendOutcome = out.withLock { o in
            if o.closed { return .noLink }
            if !o.pending.isEmpty && o.pending.count + bytes.count > capacity {
                o.wantWritable = true
                return .wouldBlock
            }
            o.pending.append(bytes)
            return .accepted
        }
        if outcome == .accepted {
            io.perform { [self] in flush() }
        }
        return outcome
    }

    public func close(reason: UInt8) {
        io.perform { [self] in teardown(.localClose) }
    }

    /// Bytes waiting for the stream (diagnostics, tests).
    public var pendingBytes: Int { out.withLock { $0.pending.count } }

    // MARK: IO thread

    private func flush() {
        guard output.streamStatus == .open || output.streamStatus == .writing else { return }
        while output.hasSpaceAvailable {
            let chunk: Data = out.withLock { $0.pending.prefix(capacity) }
            if chunk.isEmpty { break }
            let n = chunk.withUnsafeBytes { raw -> Int in
                guard let p = raw.bindMemory(to: UInt8.self).baseAddress else { return 0 }
                return output.write(p, maxLength: chunk.count)
            }
            if n < 0 {
                teardown(.error(Int32((output.streamError as NSError?)?.code ?? -1)))
                return
            }
            if n == 0 { break }
            let notify: Bool = out.withLock { o in
                o.pending.removeFirst(n)
                if o.pending.isEmpty && o.wantWritable {
                    o.wantWritable = false
                    return true
                }
                return false
            }
            if notify { registry.writable(id) }
        }
    }

    private func drainInput() {
        while input.hasBytesAvailable {
            let n = input.read(&readBuffer, maxLength: readBuffer.count)
            if n > 0 {
                registry.data(id, Data(readBuffer[0..<n]))
            } else if n < 0 {
                teardown(.error(Int32((input.streamError as NSError?)?.code ?? -1)))
                return
            } else {
                break
            }
        }
    }

    private func teardown(_ reason: LinkDownReason) {
        let first = out.withLock { o -> Bool in
            let was = o.closed
            o.closed = true
            o.pending.removeAll()
            return !was
        }
        guard first else { return }
        for s in [input as Stream, output as Stream] {
            s.delegate = nil
            s.close()
            s.remove(from: .current, forMode: .default)
        }
        registry.down(id, reason: reason)
    }

    public func stream(_ aStream: Stream, handle eventCode: Stream.Event) {
        switch eventCode {
        case .hasBytesAvailable:
            drainInput()
        case .hasSpaceAvailable:
            flush()
        case .endEncountered:
            if aStream === input { drainInput() }
            teardown(.peerClosed)
        case .errorOccurred:
            teardown(.error(Int32((aStream.streamError as NSError?)?.code ?? -1)))
        default:
            break
        }
    }
}
