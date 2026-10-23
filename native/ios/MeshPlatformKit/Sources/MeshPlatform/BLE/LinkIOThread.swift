import Foundation
import Synchronization

/// A dedicated thread with its own run loop for the L2CAP `NSStream`s (NSStream needs a
/// run loop; DESIGN §9.5). Radio and engine threads never block on it: work is posted with
/// `perform`, which returns immediately.
public final class LinkIOThread: Sendable {
    /// State shared by the owner and the thread; the thread keeps it alive after the owner
    /// goes away until the run loop has stopped.
    private final class Core: @unchecked Sendable {
        // `runLoop` and `thread` are written once before `ready` is signalled and only read
        // afterwards; `stopped` is protected by its Mutex.
        var runLoop: CFRunLoop?
        var thread: Thread?
        let ready = DispatchSemaphore(value: 0)
        let stopped = Mutex(false)

        func main() {
            let rl = RunLoop.current
            rl.add(Port(), forMode: .default) // keeps the run loop alive without sources
            runLoop = CFRunLoopGetCurrent()
            thread = Thread.current
            ready.signal()
            while !stopped.withLock({ $0 }) {
                _ = rl.run(mode: .default, before: .distantFuture)
            }
        }
    }

    private let core = Core()

    public init(name: String = "mesh.link-io") {
        let core = self.core
        let t = Thread { core.main() }
        t.name = name
        t.qualityOfService = .userInteractive
        t.start()
        core.ready.wait()
    }

    deinit {
        stop()
    }

    /// Runs `block` on the IO thread (asynchronously, FIFO).
    public func perform(_ block: @escaping @Sendable () -> Void) {
        guard let rl = core.runLoop, !core.stopped.withLock({ $0 }) else { return }
        CFRunLoopPerformBlock(rl, CFRunLoopMode.defaultMode.rawValue, block)
        CFRunLoopWakeUp(rl)
    }

    /// True when called on the IO thread.
    public var isCurrent: Bool {
        Thread.current == core.thread
    }

    /// Schedules streams on the IO thread's run loop (call from the IO thread).
    public func schedule(_ streams: [Stream]) {
        precondition(isCurrent, "streams must be scheduled on the link IO thread")
        for s in streams {
            s.schedule(in: .current, forMode: .default)
        }
    }

    public var isStopped: Bool { core.stopped.withLock { $0 } }

    /// Stops the run loop; pending blocks are dropped.
    public func stop() {
        let wasStopped = core.stopped.withLock { s -> Bool in
            let w = s
            s = true
            return w
        }
        guard !wasStopped, let rl = core.runLoop else { return }
        CFRunLoopPerformBlock(rl, CFRunLoopMode.defaultMode.rawValue) { CFRunLoopStop(rl) }
        CFRunLoopWakeUp(rl)
    }
}
