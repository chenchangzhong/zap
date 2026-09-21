// 像素采样器:为"Cef 是否真的渲染在洞里"提供可编程证据(无需视觉模型)。
// 用法: sample_pixel <png> <x,y> [<x,y> ...]   (图像像素坐标,左上原点)
// 输出: 每点一行 "<x>,<y> = #RRGGBB"
import Foundation
import CoreGraphics
import ImageIO

let args = CommandLine.arguments
guard args.count >= 3 else {
    FileHandle.standardError.write("usage: sample_pixel <png> <x,y> [...]\n".data(using: .utf8)!)
    exit(2)
}
guard let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: args[1]) as CFURL, nil),
      let image = CGImageSourceCreateImageAtIndex(src, 0, nil) else {
    FileHandle.standardError.write("cannot read image\n".data(using: .utf8)!)
    exit(2)
}

let width = image.width
let height = image.height
var pixels = [UInt8](repeating: 0, count: width * height * 4)
let space = CGColorSpaceCreateDeviceRGB()
guard let ctx = CGContext(data: &pixels, width: width, height: height,
                          bitsPerComponent: 8, bytesPerRow: width * 4, space: space,
                          bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
    FileHandle.standardError.write("cannot create context\n".data(using: .utf8)!)
    exit(2)
}
ctx.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))

print("image=\(width)x\(height)")
for spec in args.dropFirst(2) {
    let parts = spec.split(separator: ",").compactMap { Int($0) }
    guard parts.count == 2, parts[0] >= 0, parts[1] >= 0, parts[0] < width, parts[1] < height else {
        print("\(spec) = out-of-range")
        continue
    }
    let idx = (parts[1] * width + parts[0]) * 4
    let r = pixels[idx], g = pixels[idx + 1], b = pixels[idx + 2]
    print(String(format: "%d,%d = #%02X%02X%02X", parts[0], parts[1], r, g, b))
}
