// typer <delaySec> <codes>: codes "4,31s,36" (s = with Shift). Posts key events like a keyboard: the app maps
// keycode → char with its live layout (an explicit string here would freeze this process's stale layout).
import Foundation
import CoreGraphics
let delay = Double(CommandLine.arguments[1])!
let src = CGEventSource(stateID: .hidSystemState)
for item in CommandLine.arguments[2].split(separator: ",") {
    let shift = item.hasSuffix("s")
    let code = UInt16(shift ? item.dropLast() : item)!
    for down in [true, false] {
        let e = CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: down)!
        e.flags = shift ? .maskShift : []
        e.post(tap: .cghidEventTap)
        usleep(down ? 12000 : useconds_t(delay * 1_000_000))
    }
}
