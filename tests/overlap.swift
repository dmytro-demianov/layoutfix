// overlap <gapSec> <codes>: fast human chord — every key goes down before any is released ("ns " in one roll)
import Foundation
import CoreGraphics
let gap = useconds_t(Double(CommandLine.arguments[1])! * 1_000_000)
let codes = CommandLine.arguments[2].split(separator: ",").map { UInt16($0)! }
let src = CGEventSource(stateID: .hidSystemState)
func post(_ c: UInt16, _ down: Bool) { CGEvent(keyboardEventSource: src, virtualKey: c, keyDown: down)!.post(tap: .cghidEventTap) }
for c in codes { post(c, true); usleep(gap) }
for c in codes { post(c, false); usleep(gap) }
