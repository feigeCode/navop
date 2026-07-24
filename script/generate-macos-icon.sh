#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SOURCE_PNG="${1:-${PROJECT_DIR}/resources/navop-icon.png}"
OUTPUT_ICNS="${2:-${PROJECT_DIR}/resources/macos/Navop.icns}"
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/onetcli-icon.XXXXXX")"
ICONSET_DIR="${WORK_DIR}/Navop.iconset"
SWIFT_SCRIPT="${WORK_DIR}/generate-iconset.swift"
TARGET_MASTER_SIZE=1024
# Keep the rounded-square artwork visually aligned with standard macOS app
# icons instead of letting it fill almost the entire 1024px canvas.
TARGET_PADDING=80

cleanup() {
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

if [ ! -f "$SOURCE_PNG" ]; then
    echo "Error: PNG source not found at ${SOURCE_PNG}"
    exit 1
fi

mkdir -p "$ICONSET_DIR"
mkdir -p "$(dirname "$OUTPUT_ICNS")"

echo "Generating macOS icon from ${SOURCE_PNG}..."

cat > "$SWIFT_SCRIPT" <<'SWIFT'
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let arguments = CommandLine.arguments
guard arguments.count == 5 else {
    fputs("Usage: generate-iconset.swift <source-png> <iconset-dir> <master-size> <padding>\n", stderr)
    exit(1)
}

let sourceURL = URL(fileURLWithPath: arguments[1])
let iconsetDirectoryURL = URL(fileURLWithPath: arguments[2], isDirectory: true)
guard let masterSize = Int(arguments[3]), masterSize > 0 else {
    fputs("Invalid master size.\n", stderr)
    exit(1)
}
guard let padding = Int(arguments[4]), padding >= 0, padding * 2 < masterSize else {
    fputs("Invalid padding.\n", stderr)
    exit(1)
}

let iconOutputs = [
    (16, "icon_16x16.png"),
    (32, "icon_16x16@2x.png"),
    (32, "icon_32x32.png"),
    (64, "icon_32x32@2x.png"),
    (128, "icon_128x128.png"),
    (256, "icon_128x128@2x.png"),
    (256, "icon_256x256.png"),
    (512, "icon_256x256@2x.png"),
    (512, "icon_512x512.png"),
    (1024, "icon_512x512@2x.png"),
]

guard let source = CGImageSourceCreateWithURL(sourceURL as CFURL, nil),
      let sourceImage = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
    fputs("Failed to load source PNG.\n", stderr)
    exit(1)
}

let width = sourceImage.width
let height = sourceImage.height
let colorSpace = CGColorSpaceCreateDeviceRGB()
let bytesPerPixel = 4
let bytesPerRow = width * bytesPerPixel
var pixels = [UInt8](repeating: 0, count: height * bytesPerRow)

guard let scanContext = CGContext(
    data: &pixels,
    width: width,
    height: height,
    bitsPerComponent: 8,
    bytesPerRow: bytesPerRow,
    space: colorSpace,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
) else {
    fputs("Failed to create scan context.\n", stderr)
    exit(1)
}

scanContext.draw(sourceImage, in: CGRect(x: 0, y: 0, width: width, height: height))

var minX = width
var minY = height
var maxX = -1
var maxY = -1

for y in 0..<height {
    for x in 0..<width {
        let alpha = pixels[y * bytesPerRow + x * bytesPerPixel + 3]
        if alpha > 0 {
            minX = min(minX, x)
            minY = min(minY, y)
            maxX = max(maxX, x)
            maxY = max(maxY, y)
        }
    }
}

guard maxX >= minX, maxY >= minY else {
    fputs("Source PNG is fully transparent.\n", stderr)
    exit(1)
}

let cropRect = CGRect(
    x: minX,
    y: minY,
    width: maxX - minX + 1,
    height: maxY - minY + 1
)

guard let croppedImage = sourceImage.cropping(to: cropRect) else {
    fputs("Failed to crop source PNG.\n", stderr)
    exit(1)
}

func makeContext(size: Int) -> CGContext? {
    CGContext(
        data: nil,
        width: size,
        height: size,
        bitsPerComponent: 8,
        bytesPerRow: size * bytesPerPixel,
        space: colorSpace,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )
}

func writePNG(_ image: CGImage, to url: URL) -> Bool {
    guard let destination = CGImageDestinationCreateWithURL(
        url as CFURL,
        UTType.png.identifier as CFString,
        1,
        nil
    ) else {
        return false
    }
    CGImageDestinationAddImage(destination, image, nil)
    return CGImageDestinationFinalize(destination)
}

let availableSide = CGFloat(masterSize - padding * 2)
let widthScale = availableSide / CGFloat(croppedImage.width)
let heightScale = availableSide / CGFloat(croppedImage.height)
let scale = min(widthScale, heightScale)
let targetWidth = CGFloat(croppedImage.width) * scale
let targetHeight = CGFloat(croppedImage.height) * scale
let originX = (CGFloat(masterSize) - targetWidth) / 2
let originY = (CGFloat(masterSize) - targetHeight) / 2

guard let masterContext = makeContext(size: masterSize) else {
    fputs("Failed to create master context.\n", stderr)
    exit(1)
}

masterContext.interpolationQuality = .high
masterContext.clear(CGRect(x: 0, y: 0, width: masterSize, height: masterSize))
masterContext.draw(
    croppedImage,
    in: CGRect(x: originX, y: originY, width: targetWidth, height: targetHeight)
)

guard let masterImage = masterContext.makeImage() else {
    fputs("Failed to create master image.\n", stderr)
    exit(1)
}

for (size, name) in iconOutputs {
    guard let iconContext = makeContext(size: size) else {
        fputs("Failed to create icon context.\n", stderr)
        exit(1)
    }

    iconContext.interpolationQuality = .high
    iconContext.clear(CGRect(x: 0, y: 0, width: size, height: size))
    iconContext.draw(masterImage, in: CGRect(x: 0, y: 0, width: size, height: size))

    guard let iconImage = iconContext.makeImage() else {
        fputs("Failed to create resized icon image.\n", stderr)
        exit(1)
    }

    let destinationURL = iconsetDirectoryURL.appendingPathComponent(name)
    guard writePNG(iconImage, to: destinationURL) else {
        fputs("Failed to write \(name).\n", stderr)
        exit(1)
    }
}
SWIFT

CLANG_MODULE_CACHE_PATH="${CLANG_MODULE_CACHE_PATH:-${TMPDIR:-/tmp}/clang-cache}" \
    swift "$SWIFT_SCRIPT" "$SOURCE_PNG" "$ICONSET_DIR" "$TARGET_MASTER_SIZE" "$TARGET_PADDING"

iconutil -c icns "$ICONSET_DIR" -o "$OUTPUT_ICNS"

echo "Generated ${OUTPUT_ICNS}"
