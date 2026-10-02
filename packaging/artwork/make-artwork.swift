// Draws Flasher's artwork as PNGs: the app icon (macOS style, and a flat
// one for Windows and Linux) at every size it ships in, the background of
// the macOS disk image, and the side images of the Windows installer.
//
// Everything is vector drawing, done again at each size, so small icons stay
// sharp. Run through make-artwork.sh, which turns the PNGs into .icns, .ico,
// the Linux icon theme and the rest; the results are committed, so building
// packages never needs this script (or a Mac).
//
//   swift make-artwork.swift <out-dir> <font.ttf>

import CoreGraphics
import CoreText
import Foundation
import ImageIO
import UniformTypeIdentifiers

let args = CommandLine.arguments
guard args.count == 3 else {
    FileHandle.standardError.write("usage: make-artwork.swift <out-dir> <font.ttf>\n".data(using: .utf8)!)
    exit(2)
}
let outDir = URL(fileURLWithPath: args[1])
try FileManager.default.createDirectory(at: outDir, withIntermediateDirectories: true)

// MARK: - Palette

func rgb(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(
        srgbRed: CGFloat((hex >> 16) & 0xff) / 255,
        green: CGFloat((hex >> 8) & 0xff) / 255,
        blue: CGFloat(hex & 0xff) / 255,
        alpha: alpha)
}

/// The brand gradient, top-left to bottom-right: the app's blue into violet.
let brandA = rgb(0x3D7BFF)
let brandB = rgb(0x7B5CFA)
let ink = rgb(0x1C2142)
let inkSoft = rgb(0x5B6283)

// MARK: - Font

let fontData = try Data(contentsOf: URL(fileURLWithPath: args[2])) as CFData
guard let descriptor = CTFontManagerCreateFontDescriptorFromData(fontData) else {
    fatalError("cannot read the font")
}
func font(_ size: CGFloat) -> CTFont { CTFontCreateWithFontDescriptor(descriptor, size, nil) }

// MARK: - Drawing helpers

func bitmap(_ w: Int, _ h: Int) -> CGContext {
    let ctx = CGContext(
        data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.setShouldAntialias(true)
    ctx.interpolationQuality = .high
    return ctx
}

func save(_ ctx: CGContext, _ name: String) {
    let url = outDir.appendingPathComponent(name)
    let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(dest, ctx.makeImage()!, nil)
    guard CGImageDestinationFinalize(dest) else { fatalError("cannot write \(name)") }
}

/// A classic 24-bit, bottom-up BMP: what the Windows installer reads.
/// The context must be opaque.
func saveBMP(_ ctx: CGContext, _ name: String) {
    let w = ctx.width, h = ctx.height
    let src = ctx.data!.assumingMemoryBound(to: UInt8.self)
    let stride = (w * 3 + 3) & ~3
    var out = Data()
    func u16(_ v: Int) { out.append(contentsOf: [UInt8(v & 0xff), UInt8(v >> 8 & 0xff)]) }
    func u32(_ v: Int) { u16(v & 0xffff); u16(v >> 16 & 0xffff) }
    out.append(contentsOf: [0x42, 0x4d])  // "BM"
    u32(54 + stride * h); u32(0); u32(54)
    u32(40); u32(w); u32(h); u16(1); u16(24); u32(0); u32(stride * h); u32(2835); u32(2835); u32(0); u32(0)
    // CGContext rows run top to bottom in memory; BMP rows bottom to top.
    for row in (0..<h).reversed() {
        let line = src + row * ctx.bytesPerRow
        var bytes = [UInt8](repeating: 0, count: stride)
        for x in 0..<w {
            bytes[x * 3] = line[x * 4 + 2]  // B
            bytes[x * 3 + 1] = line[x * 4 + 1]  // G
            bytes[x * 3 + 2] = line[x * 4]  // R
        }
        out.append(contentsOf: bytes)
    }
    try! out.write(to: outDir.appendingPathComponent(name))
}

func linear(_ ctx: CGContext, _ colors: [CGColor], from: CGPoint, to: CGPoint) {
    let g = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB), colors: colors as CFArray, locations: nil)!
    ctx.drawLinearGradient(g, start: from, end: to, options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
}

func radial(_ ctx: CGContext, _ color: CGColor, at c: CGPoint, radius: CGFloat) {
    let clear = color.copy(alpha: 0)!
    let g = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB), colors: [color, clear] as CFArray, locations: nil)!
    ctx.drawRadialGradient(g, startCenter: c, startRadius: 0, endCenter: c, endRadius: radius, options: [])
}

func roundedRect(_ r: CGRect, _ radius: CGFloat) -> CGPath {
    CGPath(roundedRect: r, cornerWidth: radius, cornerHeight: radius, transform: nil)
}

/// A line of text, centred on `x`, its baseline at `y`.
func text(_ ctx: CGContext, _ s: String, size: CGFloat, color: CGColor, x: CGFloat, y: CGFloat, tracking: CGFloat = 0) {
    let attrs: [CFString: Any] = [
        kCTFontAttributeName: font(size),
        kCTForegroundColorAttributeName: color,
        kCTKernAttributeName: tracking,
    ]
    let line = CTLineCreateWithAttributedString(NSAttributedString(string: s, attributes: attrs as [NSAttributedString.Key: Any]))
    let width = CTLineGetTypographicBounds(line, nil, nil, nil)
    ctx.textPosition = CGPoint(x: x - width / 2, y: y)
    CTLineDraw(line, ctx)
}

// MARK: - The glyph: a USB stick with a lightning bolt

/// Drawn in a 1024-unit square centred on (512, 512); the caller scales.
func drawGlyph(_ ctx: CGContext, boltColors: [CGColor]) {
    // Connector, behind the body.
    let connector = CGRect(x: 412, y: 640, width: 200, height: 176)
    ctx.addPath(roundedRect(connector, 26))
    ctx.setFillColor(rgb(0xE4E8FB))
    ctx.fillPath()
    for dx in [-46.0, 46.0] {
        ctx.addPath(roundedRect(CGRect(x: 512 + dx - 22, y: 742, width: 44, height: 40), 8))
        ctx.setFillColor(brandB.copy(alpha: 0.85)!)
        ctx.fillPath()
    }

    // Body, with a soft shadow under it.
    let body = CGRect(x: 357, y: 214, width: 310, height: 452)
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -14), blur: 36, color: rgb(0x0B1240, 0.35))
    ctx.addPath(roundedRect(body, 78))
    ctx.setFillColor(rgb(0xFFFFFF))
    ctx.fillPath()
    ctx.restoreGState()

    // The bolt, in the brand gradient.
    let bolt = CGMutablePath()
    let pts: [(CGFloat, CGFloat)] = [
        (542, 604), (420, 426), (497, 426), (466, 268), (604, 470), (523, 470),
    ]
    bolt.move(to: CGPoint(x: pts[0].0, y: pts[0].1))
    for p in pts.dropFirst() { bolt.addLine(to: CGPoint(x: p.0, y: p.1)) }
    bolt.closeSubpath()
    ctx.saveGState()
    ctx.addPath(bolt)
    ctx.clip()
    linear(ctx, boltColors, from: CGPoint(x: 420, y: 604), to: CGPoint(x: 604, y: 268))
    ctx.restoreGState()
}

// MARK: - App icon

enum Style { case mac, flat }

/// macOS: Apple's grid, an 824-unit rounded square with a shadow inside a
/// 1024 canvas. Flat (Windows, Linux): the square fills the canvas.
func icon(size: Int, style: Style) -> CGContext {
    let ctx = bitmap(size, size)
    let s = CGFloat(size) / 1024
    ctx.scaleBy(x: s, y: s)

    let tile: CGRect
    let radius: CGFloat
    switch style {
    case .mac: tile = CGRect(x: 100, y: 100, width: 824, height: 824); radius = 185
    case .flat: tile = CGRect(x: 40, y: 40, width: 944, height: 944); radius = 200
    }

    if style == .mac {
        ctx.saveGState()
        ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 28, color: rgb(0x000000, 0.30))
        ctx.addPath(roundedRect(tile, radius))
        ctx.setFillColor(brandA)
        ctx.fillPath()
        ctx.restoreGState()
    }

    ctx.saveGState()
    ctx.addPath(roundedRect(tile, radius))
    ctx.clip()
    linear(ctx, [brandA, brandB], from: CGPoint(x: tile.minX, y: tile.maxY), to: CGPoint(x: tile.maxX, y: tile.minY))
    // Light from above: a glow at the top, a little depth at the bottom.
    radial(ctx, rgb(0xFFFFFF, 0.28), at: CGPoint(x: tile.midX - 120, y: tile.maxY), radius: 620)
    radial(ctx, rgb(0x1A0B5C, 0.22), at: CGPoint(x: tile.maxX, y: tile.minY), radius: 520)
    ctx.restoreGState()

    // The glyph, scaled to the tile.
    ctx.saveGState()
    let g = tile.width / 824
    ctx.translateBy(x: 512, y: 512)
    ctx.scaleBy(x: g, y: g)
    ctx.translateBy(x: -512, y: -512)
    drawGlyph(ctx, boltColors: [brandA, brandB])
    ctx.restoreGState()
    return ctx
}

for size in [16, 32, 64, 128, 256, 512, 1024] {
    save(icon(size: size, style: .mac), "icon-mac-\(size).png")
}
for size in [16, 20, 24, 32, 40, 48, 64, 128, 256, 512] {
    save(icon(size: size, style: .flat), "icon-flat-\(size).png")
}

// MARK: - Disk image background

/// The Finder window of the disk image, 660 × 420 points: the app on the
/// left, Applications on the right (positions set in package-macos.sh).
func dmgBackground(scale: CGFloat) -> CGContext {
    let w = 660.0, h = 420.0
    let ctx = bitmap(Int(w * scale), Int(h * scale))
    ctx.scaleBy(x: scale, y: scale)

    linear(ctx, [rgb(0xF8F9FF), rgb(0xE9EDFF)], from: CGPoint(x: 0, y: h), to: CGPoint(x: 0, y: 0))
    radial(ctx, brandB.copy(alpha: 0.20)!, at: CGPoint(x: 600, y: 400), radius: 300)
    radial(ctx, brandA.copy(alpha: 0.16)!, at: CGPoint(x: 40, y: 30), radius: 280)
    radial(ctx, rgb(0xFFFFFF, 0.7), at: CGPoint(x: 330, y: 230), radius: 220)

    text(ctx, "Flasher", size: 30, color: ink, x: w / 2, y: h - 66, tracking: -0.4)
    text(ctx, "Write disk images to USB drives and SD cards", size: 13, color: inkSoft, x: w / 2, y: h - 90)

    // Soft waves along the bottom.
    for (i, (color, alpha)) in [(brandA, 0.10), (brandB, 0.09), (brandA, 0.07)].enumerated() {
        let base = 70.0 - Double(i) * 22
        let wave = CGMutablePath()
        wave.move(to: CGPoint(x: 0, y: 0))
        wave.addLine(to: CGPoint(x: 0, y: base + 18))
        wave.addCurve(
            to: CGPoint(x: w, y: base + Double(i) * 10),
            control1: CGPoint(x: 220 + Double(i) * 60, y: base + 70),
            control2: CGPoint(x: 430 - Double(i) * 40, y: base - 40))
        wave.addLine(to: CGPoint(x: w, y: 0))
        wave.closeSubpath()
        ctx.addPath(wave)
        ctx.setFillColor(color.copy(alpha: alpha)!)
        ctx.fillPath()
    }

    // A dotted arc from the app to Applications, ending in an arrowhead
    // that follows it. Icon centres are 205 points up, at x = 165 and 495.
    let y = 205.0
    let start = CGPoint(x: 250, y: y + 8)
    let control = CGPoint(x: 330, y: y + 52)
    let end = CGPoint(x: 404, y: y + 10)
    let arc = CGMutablePath()
    arc.move(to: start)
    arc.addQuadCurve(to: end, control: control)
    ctx.saveGState()
    ctx.addPath(arc)
    ctx.setLineWidth(3.5)
    ctx.setLineCap(.round)
    ctx.setLineDash(phase: 0, lengths: [0.1, 10])
    ctx.setStrokeColor(brandB.copy(alpha: 0.75)!)
    ctx.strokePath()
    ctx.restoreGState()
    // The curve's direction at its end is end − control.
    let dx = end.x - control.x, dy = end.y - control.y
    let len = (dx * dx + dy * dy).squareRoot()
    let (ux, uy) = (dx / len, dy / len)
    let tip = CGPoint(x: end.x + ux * 10, y: end.y + uy * 10)
    let head = CGMutablePath()
    for angle in [0.6, -0.6] {
        let (c, s) = (cos(angle), sin(angle))
        // Back along the curve, turned by ±angle.
        let bx = -(ux * c - uy * s), by = -(ux * s + uy * c)
        head.move(to: tip)
        head.addLine(to: CGPoint(x: tip.x + bx * 17, y: tip.y + by * 17))
    }
    ctx.addPath(head)
    ctx.setLineWidth(3.5)
    ctx.setLineCap(.round)
    ctx.setStrokeColor(brandB.copy(alpha: 0.85)!)
    ctx.strokePath()

    text(ctx, "Drag Flasher onto Applications to install it", size: 13, color: inkSoft, x: w / 2, y: 54)
    return ctx
}

save(dmgBackground(scale: 1), "dmg-background.png")
save(dmgBackground(scale: 2), "dmg-background@2x.png")

// MARK: - Windows installer images

/// The tall image on the installer's first and last pages (164 × 314 at
/// 100 % scaling), opaque.
func wizardLarge(scale: CGFloat) -> CGContext {
    let w = 164.0, h = 314.0
    let ctx = bitmap(Int(w * scale), Int(h * scale))
    ctx.scaleBy(x: scale, y: scale)
    linear(ctx, [brandA, brandB], from: CGPoint(x: 0, y: h), to: CGPoint(x: w, y: 0))
    radial(ctx, rgb(0xFFFFFF, 0.25), at: CGPoint(x: 30, y: h), radius: 260)
    radial(ctx, rgb(0x1A0B5C, 0.25), at: CGPoint(x: w, y: 0), radius: 220)
    ctx.saveGState()
    let g = 120.0 / 824
    ctx.translateBy(x: w / 2, y: h - 120)
    ctx.scaleBy(x: g, y: g)
    ctx.translateBy(x: -512, y: -512)
    drawGlyph(ctx, boltColors: [brandA, brandB])
    ctx.restoreGState()
    text(ctx, "Flasher", size: 22, color: rgb(0xFFFFFF), x: w / 2, y: 92, tracking: -0.2)
    return ctx
}

/// The small image at the top right of the other pages (55 × 55), opaque.
func wizardSmall(scale: CGFloat) -> CGContext {
    let s = 55.0
    let ctx = bitmap(Int(s * scale), Int(s * scale))
    ctx.scaleBy(x: scale, y: scale)
    ctx.setFillColor(rgb(0xFFFFFF))
    ctx.fill(CGRect(x: 0, y: 0, width: s, height: s))
    let img = icon(size: Int(s * scale), style: .flat).makeImage()!
    ctx.draw(img, in: CGRect(x: 0, y: 0, width: s, height: s))
    return ctx
}

for scale in [1.0, 2.0] {
    for (ctx, name) in [(wizardLarge(scale: scale), "wizard-large"), (wizardSmall(scale: scale), "wizard-small")] {
        save(ctx, "\(name)-\(Int(scale * 100)).png")
        saveBMP(ctx, "\(name)-\(Int(scale * 100)).bmp")
    }
}
