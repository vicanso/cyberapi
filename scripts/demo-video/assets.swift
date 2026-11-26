// scripts/demo-video/assets.swift
// Renders light-mode studio layers used by compose.py:
//   frame   <out.png> [winW winH winOX winOY]  3200x1800 light backdrop with rounded hole + soft shadow
//   card    <intro|outro> <out.png> <icon>     1920x1080 light title / closing cards
//   caption <out.png> <en> <zh> <index>        1920x1080 frosted lower-third caption

import AppKit
import Foundation

let args = CommandLine.arguments
guard args.count >= 3 else {
    FileHandle.standardError.write("usage: assets frame|card|caption ...\n".data(using: .utf8)!)
    exit(2)
}

let canvasW = 3200, canvasH = 1800
let winRadius: CGFloat = 30 // matches macOS Retina rounded window corners cleanly

func rgb(_ hex: UInt32, _ a: CGFloat = 1) -> NSColor {
    NSColor(
        srgbRed: CGFloat((hex >> 16) & 0xff) / 255,
        green: CGFloat((hex >> 8) & 0xff) / 255,
        blue: CGFloat(hex & 0xff) / 255,
        alpha: a
    )
}

func render(_ w: Int, _ h: Int, to path: String, _ draw: (NSRect) -> Void) {
    guard let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: w,
        pixelsHigh: h,
        bitsPerSample: 8,
        samplesPerPixel: 4,
        hasAlpha: true,
        isPlanar: false,
        colorSpaceName: .deviceRGB,
        bytesPerRow: 0,
        bitsPerPixel: 0
    ) else { exit(1) }
    rep.size = NSSize(width: w, height: h)
    NSGraphicsContext.saveGraphicsState()
    let ctx = NSGraphicsContext(bitmapImageRep: rep)!
    NSGraphicsContext.current = ctx
    ctx.imageInterpolation = .high
    NSColor.clear.set()
    NSRect(x: 0, y: 0, width: w, height: h).fill(using: .copy)
    draw(NSRect(x: 0, y: 0, width: w, height: h))
    NSGraphicsContext.restoreGraphicsState()
    try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: path))
}

func backdrop(_ r: NSRect) {
    NSGradient(
        colors: [rgb(0xf8fafc), rgb(0xf1f5f9), rgb(0xe6effa)],
        atLocations: [0, 0.55, 1],
        colorSpace: .sRGB
    )!.draw(in: r, angle: -35)

    // Soft ambient studio glows echoing CyberAPI's cyan/cobalt/amber icon palette.
    let glows: [(NSPoint, CGFloat, NSColor)] = [
        (NSPoint(x: r.width * 0.16, y: r.height * 0.86), r.width * 0.42, rgb(0x06b6d4, 0.13)),
        (NSPoint(x: r.width * 0.86, y: r.height * 0.14), r.width * 0.40, rgb(0x3b82f6, 0.11)),
        (NSPoint(x: r.width * 0.84, y: r.height * 0.88), r.width * 0.30, rgb(0xf59e0b, 0.06)),
    ]
    for (c, radius, color) in glows {
        NSGradient(starting: color, ending: color.withAlphaComponent(0))!
            .draw(fromCenter: c, radius: 0, toCenter: c, radius: radius, options: [])
    }

    // Subtle precision dot grid.
    rgb(0x0f172a, 0.04).setFill()
    let step = r.width / 64
    var y = step / 2
    while y < r.height {
        var x = step / 2
        while x < r.width {
            NSBezierPath(ovalIn: NSRect(x: x - 1.5, y: y - 1.5, width: 3, height: 3)).fill()
            x += step
        }
        y += step
    }
}

func drawText(_ s: String, font: NSFont, color: NSColor, centerX: CGFloat, y: CGFloat, kern: CGFloat = 0) -> NSSize {
    let attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color, .kern: kern]
    let str = NSAttributedString(string: s, attributes: attrs)
    let size = str.size()
    str.draw(at: NSPoint(x: centerX - size.width / 2, y: y))
    return size
}

func pill(_ label: String, font: NSFont, at origin: NSPoint, fill: NSColor, stroke: NSColor, text: NSColor) -> CGFloat {
    let attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: text]
    let str = NSAttributedString(string: label, attributes: attrs)
    let sz = str.size()
    let h = sz.height + 22
    let rect = NSRect(x: origin.x, y: origin.y, width: sz.width + 44, height: h)
    let path = NSBezierPath(roundedRect: rect, xRadius: h / 2, yRadius: h / 2)
    fill.setFill()
    path.fill()
    stroke.setStroke()
    path.lineWidth = 1.5
    path.stroke()
    str.draw(at: NSPoint(x: rect.minX + 22, y: rect.minY + 11))
    return rect.width
}

func pillsRow(_ labels: [String], font: NSFont, centerX: CGFloat, y: CGFloat) {
    let attrs: [NSAttributedString.Key: Any] = [.font: font]
    let gap: CGFloat = 14
    let widths = labels.map { NSAttributedString(string: $0, attributes: attrs).size().width + 44 }
    var x = centerX - (widths.reduce(0, +) + gap * CGFloat(labels.count - 1)) / 2
    for (i, l) in labels.enumerated() {
        _ = pill(
            l,
            font: font,
            at: NSPoint(x: x, y: y),
            fill: rgb(0xffffff, 0.92),
            stroke: rgb(0xcbd5e1, 0.95),
            text: rgb(0x1e293b)
        )
        x += widths[i] + gap
    }
}

let mode = args[1]
switch mode {
case "frame":
    let winW = args.count > 3 ? (CGFloat(Double(args[3]) ?? 2560)) : 2560
    let winH = args.count > 4 ? (CGFloat(Double(args[4]) ?? 1440)) : 1440
    let winOX = args.count > 5 ? (CGFloat(Double(args[5]) ?? 320)) : 320
    let winOY = args.count > 6 ? (CGFloat(Double(args[6]) ?? 110)) : 110
    let winRect = NSRect(x: winOX, y: CGFloat(canvasH) - winOY - winH, width: winW, height: winH)

    render(canvasW, canvasH, to: args[2]) { r in
        backdrop(r)
        let hole = NSBezierPath(roundedRect: winRect, xRadius: winRadius, yRadius: winRadius)

        NSGraphicsContext.saveGraphicsState()
        let shadow = NSShadow()
        shadow.shadowColor = rgb(0x0f172a, 0.22)
        shadow.shadowBlurRadius = 72
        shadow.shadowOffset = NSSize(width: 0, height: -24)
        shadow.set()
        NSColor.white.setFill()
        hole.fill()
        NSGraphicsContext.restoreGraphicsState()

        rgb(0x0f172a, 0.06).setStroke()
        let edge = NSBezierPath(
            roundedRect: winRect.insetBy(dx: -0.75, dy: -0.75),
            xRadius: winRadius + 0.75,
            yRadius: winRadius + 0.75
        )
        edge.lineWidth = 1.5
        edge.stroke()

        NSGraphicsContext.current?.compositingOperation = .copy
        NSColor.clear.setFill()
        hole.fill()
    }

case "card":
    let kind = args[2], out = args[3], iconPath = args[4]
    render(1920, 1080, to: out) { r in
        backdrop(r)
        let icon = NSImage(contentsOfFile: iconPath)
        if kind == "intro" {
            icon?.draw(in: NSRect(x: 960 - 80, y: 645, width: 160, height: 160))
            _ = drawText("Cyber API", font: .systemFont(ofSize: 92, weight: .heavy), color: rgb(0x0f172a), centerX: 960, y: 515, kern: -1)
            _ = drawText(
                "A fast, native API client in Rust & GPUI — desktop and self-hosted web",
                font: .systemFont(ofSize: 32, weight: .medium),
                color: rgb(0x334155),
                centerX: 960,
                y: 446
            )
            _ = drawText(
                "基于 Rust 与 GPUI 构建的原生高速 API 客户端 · 桌面版 & 自托管网页版",
                font: .systemFont(ofSize: 28, weight: .regular),
                color: rgb(0x64748b),
                centerX: 960,
                y: 394
            )
            pillsRow(
                [
                    "REST · GraphQL · gRPC · WS · SSE",
                    "Assertions & JSON Tree · 断言与树视图",
                    "OpenAPI Sync · 文档同步",
                    "Desktop · Web · MCP",
                ],
                font: .systemFont(ofSize: 21, weight: .medium),
                centerX: 960,
                y: 272
            )
        } else {
            icon?.draw(in: NSRect(x: 960 - 64, y: 655, width: 128, height: 128))
            _ = drawText("Cyber API", font: .systemFont(ofSize: 82, weight: .heavy), color: rgb(0x0f172a), centerX: 960, y: 535, kern: -1)
            _ = drawText(
                "github.com/vicanso/cyberapi",
                font: .monospacedSystemFont(ofSize: 36, weight: .semibold),
                color: rgb(0x2563eb),
                centerX: 960,
                y: 455
            )
            _ = drawText(
                "macOS · Linux · Windows · Self-hosted Web (Docker / Binary)",
                font: .systemFont(ofSize: 26, weight: .regular),
                color: rgb(0x475569),
                centerX: 960,
                y: 395
            )
            let demoLabel = "Live web demo: cyberapi.vicanso.dev"
            let font = NSFont.monospacedSystemFont(ofSize: 24, weight: .medium)
            let w = NSAttributedString(string: demoLabel, attributes: [.font: font]).size().width + 44
            _ = pill(
                demoLabel,
                font: font,
                at: NSPoint(x: 960 - w / 2, y: 282),
                fill: rgb(0xffffff, 0.96),
                stroke: rgb(0xcbd5e1),
                text: rgb(0x0f172a)
            )
        }
    }

case "caption":
    let out = args[2], en = args[3], zh = args[4], index = args.count > 5 ? args[5] : ""
    render(1920, 1080, to: out) { _ in
        let enFont = NSFont.systemFont(ofSize: 32, weight: .semibold)
        let zhFont = NSFont.systemFont(ofSize: 23, weight: .regular)
        let idxFont = NSFont.monospacedDigitSystemFont(ofSize: 21, weight: .bold)
        let enStr = NSAttributedString(string: en, attributes: [.font: enFont, .foregroundColor: rgb(0x0f172a)])
        let zhStr = NSAttributedString(string: zh, attributes: [.font: zhFont, .foregroundColor: rgb(0x475569)])
        let idxStr = NSAttributedString(string: index, attributes: [.font: idxFont, .foregroundColor: rgb(0xffffff)])
        let idxW = index.isEmpty ? 0 : idxStr.size().width + 26
        let textW = max(enStr.size().width, zhStr.size().width)
        let padX: CGFloat = 28, h: CGFloat = 100
        let w = padX * 2 + textW + (idxW > 0 ? idxW + 20 : 0)
        let rect = NSRect(x: 960 - w / 2, y: 24, width: w, height: h)

        NSGraphicsContext.saveGraphicsState()
        let sh = NSShadow()
        sh.shadowColor = rgb(0x0f172a, 0.16)
        sh.shadowBlurRadius = 26
        sh.shadowOffset = NSSize(width: 0, height: -6)
        sh.set()
        rgb(0xffffff, 0.95).setFill()
        NSBezierPath(roundedRect: rect, xRadius: 20, yRadius: 20).fill()
        NSGraphicsContext.restoreGraphicsState()

        rgb(0xcbd5e1, 0.90).setStroke()
        let border = NSBezierPath(roundedRect: rect.insetBy(dx: 0.75, dy: 0.75), xRadius: 20, yRadius: 20)
        border.lineWidth = 1.5
        border.stroke()

        var x = rect.minX + padX
        if idxW > 0 {
            let badge = NSRect(x: x, y: rect.midY - 20, width: idxW, height: 40)
            NSGradient(starting: rgb(0x2563eb), ending: rgb(0x06b6d4))!
                .draw(in: NSBezierPath(roundedRect: badge, xRadius: 11, yRadius: 11), angle: 0)
            idxStr.draw(at: NSPoint(x: badge.minX + 13, y: badge.midY - idxStr.size().height / 2))
            x += idxW + 20
        }
        enStr.draw(at: NSPoint(x: x, y: rect.minY + 48))
        zhStr.draw(at: NSPoint(x: x, y: rect.minY + 16))
    }

default:
    exit(2)
}
