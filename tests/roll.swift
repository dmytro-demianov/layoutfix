// roll <gapSec> <codes>: human-like rollover — next key goes down before the previous one is released
import Foundation
import CoreGraphics
let gap = Double(CommandLine.arguments[1])!
let codes = CommandLine.arguments[2].split(separator: ",").map { UInt16($0)! }
let src = CGEventSource(stateID: .hidSystemState)
func post(_ c: UInt16, _ down: Bool) { CGEvent(keyboardEventSource: src, virtualKey: c, keyDown: down)!.post(tap: .cghidEventTap) }
var prev: UInt16? = nil
for c in codes {
    post(c, true)
    if let p = prev { usleep(3000); post(p, false) }
    prev = c
    usleep(useconds_t(gap * 1_000_000))
}
if let p = prev { usleep(30000); post(p, false) }
