import Carbon
let want = CommandLine.arguments[1]
let list = TISCreateInputSourceList([kTISPropertyInputSourceID: want] as CFDictionary, false).takeRetainedValue() as! [TISInputSource]
if let s = list.first { TISSelectInputSource(s) }
