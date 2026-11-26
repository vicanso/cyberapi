// scripts/demo-video/driver.swift
// Input driver for the CyberAPI demo recording. Moves the cursor along eased
// paths, clicks, types and scrolls, and appends every action with its host
// time to $DRV_LOG so compose.py can synchronize camera moves and captions.
//
// Coordinates are window-relative points; the window origin comes from
// $WIN_X / $WIN_Y.

import CoreGraphics
import Foundation

func hostNow() -> Double {
    var tb = mach_timebase_info_data_t()
    mach_timebase_info(&tb)
    return Double(mach_absolute_time()) * Double(tb.numer) / Double(tb.denom) / 1e9
}

let env = ProcessInfo.processInfo.environment
let winX = Double(env["WIN_X"] ?? "0") ?? 0
let winY = Double(env["WIN_Y"] ?? "0") ?? 0

func log(_ fields: [String: Any]) {
    guard let path = env["DRV_LOG"] else { return }
    var obj = fields
    obj["t"] = hostNow()
    guard let data = try? JSONSerialization.data(withJSONObject: obj, options: [.sortedKeys]),
          var line = String(data: data, encoding: .utf8) else { return }
    line += "\n"
    if let h = FileHandle(forWritingAtPath: path) {
        h.seekToEndOfFile()
        h.write(line.data(using: .utf8)!)
        h.closeFile()
    } else {
        try? line.write(toFile: path, atomically: false, encoding: .utf8)
    }
}

func cursor() -> CGPoint {
    CGEvent(source: nil)?.location ?? .zero
}

func abs(_ x: Double, _ y: Double) -> CGPoint {
    CGPoint(x: winX + x, y: winY + y)
}

func post(_ type: CGEventType, _ p: CGPoint, _ button: CGMouseButton = .left) {
    CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: p, mouseButton: button)?
        .post(tap: .cghidEventTap)
}

func glide(to target: CGPoint, duration: Double) {
    let start = cursor()
    let dx = target.x - start.x
    let dy = target.y - start.y
    let dist = (dx * dx + dy * dy).squareRoot()
    if dist < 1 { return }
    let dur = duration > 0 ? duration : min(0.95, max(0.35, dist / 1400))
    let steps = max(2, Int(dur * 120))
    let arc = min(60.0, dist * 0.08)
    let nx = -dy / dist
    let ny = dx / dist
    for i in 1...steps {
        let u = Double(i) / Double(steps)
        let e = u < 0.5 ? 4 * u * u * u : 1 - pow(-2 * u + 2, 3) / 2
        let bow = sin(Double.pi * e) * arc
        let p = CGPoint(x: start.x + dx * e + nx * bow, y: start.y + dy * e + ny * bow)
        post(.mouseMoved, p)
        usleep(useconds_t(dur / Double(steps) * 1_000_000))
    }
    post(.mouseMoved, target)
}

func click(_ p: CGPoint, right: Bool) {
    let down: CGEventType = right ? .rightMouseDown : .leftMouseDown
    let up: CGEventType = right ? .rightMouseUp : .leftMouseUp
    let btn: CGMouseButton = right ? .right : .left
    post(down, p, btn)
    usleep(70_000)
    post(up, p, btn)
}

let plainKeys: [Character: CGKeyCode] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9,
    "b": 11, "q": 12, "w": 13, "e": 14, "r": 15, "y": 16, "t": 17, "1": 18, "2": 19,
    "3": 20, "4": 21, "6": 22, "5": 23, "=": 24, "9": 25, "7": 26, "-": 27, "8": 28,
    "0": 29, "]": 30, "o": 31, "u": 32, "[": 33, "i": 34, "p": 35, "\n": 36, "l": 37,
    "j": 38, "'": 39, "k": 40, ";": 41, "\\": 42, ",": 43, "/": 44, "n": 45, "m": 46,
    ".": 47, " ": 49, "`": 50,
]
let shiftedKeys: [Character: Character] = [
    ":": ";", "|": "\\", "_": "-", "*": "8", "+": "=", "\"": "'", "<": ",", ">": ".",
    "?": "/", "!": "1", "@": "2", "#": "3", "$": "4", "%": "5", "^": "6", "&": "7",
    "(": "9", ")": "0", "{": "[", "}": "]", "~": "`",
]

func typeText(_ text: String, cps: Double) {
    for ch in text {
        var shift = false
        var base = ch
        if let b = shiftedKeys[ch] {
            shift = true
            base = b
        } else if ch.isUppercase, let lower = ch.lowercased().first {
            shift = true
            base = lower
        }
        let code = plainKeys[base] ?? 0
        let units = Array(String(ch).utf16)
        for down in [true, false] {
            if let ev = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down) {
                ev.flags = shift ? .maskShift : []
                ev.keyboardSetUnicodeString(stringLength: units.count, unicodeString: units)
                ev.post(tap: .cghidEventTap)
            }
            usleep(14_000)
        }
        let jitter = Double.random(in: 0.75...1.35)
        usleep(useconds_t(1_000_000 / cps * jitter))
    }
}

func key(_ code: CGKeyCode, _ mods: [String]) {
    var flags: CGEventFlags = []
    for m in mods {
        switch m {
        case "cmd": flags.insert(.maskCommand)
        case "shift": flags.insert(.maskShift)
        case "alt": flags.insert(.maskAlternate)
        case "ctrl": flags.insert(.maskControl)
        default: break
        }
    }
    for down in [true, false] {
        if let ev = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down) {
            ev.flags = flags
            ev.post(tap: .cghidEventTap)
        }
        usleep(30_000)
    }
}

func scroll(lines: Int, duration: Double) {
    let steps = max(1, Swift.abs(lines))
    let dir: Int32 = lines < 0 ? 1 : -1
    for _ in 0..<steps {
        CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: dir * 12, wheel2: 0, wheel3: 0)?
            .post(tap: .cghidEventTap)
        usleep(useconds_t(duration / Double(steps) * 1_000_000))
    }
}

func windowInfo(pid: Int) -> [String: Any]? {
    let opts = CGWindowListOption(arrayLiteral: .excludeDesktopElements, .optionOnScreenOnly)
    guard let list = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] else { return nil }
    let matches = list.filter { item in
        guard (item[kCGWindowOwnerPID as String] as? Int) == pid,
              (item[kCGWindowLayer as String] as? Int) == 0,
              let b = item[kCGWindowBounds as String] as? [String: Any] else { return false }
        let w = b["Width"] as? Double ?? 0
        let h = b["Height"] as? Double ?? 0
        return w > 10 && h > 10
    }
    return matches.max { a, b in
        let ab = a[kCGWindowBounds as String] as? [String: Any]
        let bb = b[kCGWindowBounds as String] as? [String: Any]
        let aArea = (ab?["Width"] as? Double ?? 0) * (ab?["Height"] as? Double ?? 0)
        let bArea = (bb?["Width"] as? Double ?? 0) * (bb?["Height"] as? Double ?? 0)
        return aArea < bArea
    }
}

var args = Array(CommandLine.arguments.dropFirst())
guard !args.isEmpty else {
    FileHandle.standardError.write("usage: driver <now|move|click|rclick|type|key|scroll|mark|cam|scale|bounds|wid> ...\n".data(using: .utf8)!)
    exit(2)
}
let cmd = args.removeFirst()
func num(_ i: Int, _ def: Double = 0) -> Double { i < args.count ? (Double(args[i]) ?? def) : def }

switch cmd {
case "now":
    print(String(format: "%.4f", hostNow()))
case "move":
    glide(to: abs(num(0), num(1)), duration: num(2))
    log(["ev": "move", "x": num(0), "y": num(1)])
case "click", "rclick":
    let p = abs(num(0), num(1))
    glide(to: p, duration: num(2))
    usleep(90_000)
    log(["ev": cmd, "x": num(0), "y": num(1)])
    click(p, right: cmd == "rclick")
case "type":
    log(["ev": "type", "text": args.first ?? ""])
    typeText(args.first ?? "", cps: num(1, 14))
case "key":
    let code = CGKeyCode(num(0))
    log(["ev": "key", "code": Int(code)])
    key(code, Array(args.dropFirst()))
case "scroll":
    log(["ev": "scroll", "lines": Int(num(0))])
    scroll(lines: Int(num(0)), duration: num(1, 0.6))
case "mark":
    log(["ev": "mark", "id": args.first ?? "", "en": args.count > 1 ? args[1] : "", "zh": args.count > 2 ? args[2] : ""])
case "cam":
    log(["ev": "cam", "zoom": num(0, 1), "x": num(1), "y": num(2), "dur": num(3, 0.9)])
case "scale":
    let id = CGMainDisplayID()
    let pts = CGDisplayBounds(id).width
    let px = CGFloat(CGDisplayCopyDisplayMode(id)?.pixelWidth ?? Int(pts))
    print(Int((px / max(pts, 1)).rounded()))
case "bounds", "wid":
    guard let w = windowInfo(pid: Int(num(0))) else { exit(1) }
    if cmd == "wid" {
        print(w[kCGWindowNumber as String] as? Int ?? 0)
    } else if let b = w[kCGWindowBounds as String] as? [String: Any] {
        let v = ["X", "Y", "Width", "Height"].map { Int((b[$0] as? Double) ?? 0) }
        print(v.map(String.init).joined(separator: ","))
    }
default:
    FileHandle.standardError.write("unknown command \(cmd)\n".data(using: .utf8)!)
    exit(2)
}
