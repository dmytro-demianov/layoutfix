// lastchord <delaySec> <gapSec> <codes>: types all keys but the last normally, then presses the last key and,
// `gap` later, space while that key is still held; releases space first, the letter ~40 ms later.
import Foundation
import CoreGraphics
let delay = useconds_t(Double(CommandLine.arguments[1])! * 1_000_000)
let gap = useconds_t(Double(CommandLine.arguments[2])! * 1_000_000)
let codes = CommandLine.arguments[3].split(separator: ",").map { UInt16($0)! }
let src = CGEventSource(stateID: .hidSystemState)
func post(_ c: UInt16, _ down: Bool) { CGEvent(keyboardEventSource: src, virtualKey: c, keyDown: down)!.post(tap: .cghidEventTap) }
for c in codes.dropLast() { post(c, true); usleep(12000); post(c, false); usleep(delay) }
let last = codes.last!
post(last, true); usleep(gap); post(49, true); usleep(5000); post(49, false); usleep(40000); post(last, false)
