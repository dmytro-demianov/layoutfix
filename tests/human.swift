// human <seed> <codes>: human-like typing — bursts of 2–5 keys a few ms apart with key rollover,
// random pauses (also mid-word), and space often hit right after the last letter. Deterministic per seed.
import Foundation
import CoreGraphics
var rng = UInt64(CommandLine.arguments[1])! &* 6364136223846793005 &+ 1442695040888963407
func rand(_ n: UInt64) -> UInt64 { rng = rng &* 6364136223846793005 &+ 1442695040888963407; return (rng >> 33) % n }
let codes = CommandLine.arguments[2].split(separator: ",").map { UInt16($0)! }
let src = CGEventSource(stateID: .hidSystemState)
func post(_ c: UInt16, _ down: Bool) { CGEvent(keyboardEventSource: src, virtualKey: c, keyDown: down)!.post(tap: .cghidEventTap) }
var held: UInt16? = nil
var burst = 0
for c in codes {
    post(c, true)
    if let h = held { usleep(useconds_t(1000 + rand(4000))); post(h, false) }   // rollover
    held = c
    if burst <= 0 { burst = Int(2 + rand(4)) }
    burst -= 1
    let ms: UInt64 = burst > 0 ? 2 + rand(14) : 60 + rand(320)                   // burst or pause
    usleep(useconds_t(ms * 1000))
}
if let h = held { usleep(20000); post(h, false) }
