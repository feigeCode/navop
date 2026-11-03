#!/usr/bin/env python3
"""生成三个产品（navop / navop-shell / navop-data）的应用图标。

三套图标共用同一套视觉语言：深蓝圆角底 + 青蓝渐变前景，靠中间的字形区分产品
（N 折线 / 终端提示符 / 数据库柱体）。全量产品的图标是既有设计，不在这里生成，
本脚本只负责两个独立产品，同时输出 Linux PNG、macOS ICNS 与 Windows ICO。

用法：
    python3 script/generate-product-icons.py [--preview-dir DIR]
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw

PROJECT_DIR = Path(__file__).resolve().parent.parent
LINUX_DIR = PROJECT_DIR / "resources" / "linux"
MACOS_DIR = PROJECT_DIR / "resources" / "macos"
WINDOWS_DIR = PROJECT_DIR / "resources" / "windows"

# 画布按 4 倍超采样再缩小，避免圆角与细线出现锯齿。
SUPERSAMPLE = 4
CANVAS = 512

BACKGROUND_TOP = (7, 18, 36)
BACKGROUND_BOTTOM = (18, 48, 92)
BORDER = (96, 214, 255, 90)
FOREGROUND_START = (94, 231, 245)
FOREGROUND_END = (63, 169, 255)
GLYPH_LIGHT = (234, 247, 255)

# 产品 -> (Linux/macOS/Windows 文件名主干, 前景绘制函数)
PRODUCTS = ("navop-shell", "navop-data")


def vertical_gradient(size: int, top: tuple[int, int, int], bottom: tuple[int, int, int]) -> Image.Image:
    """竖直渐变，按行插值后横向拉伸。"""
    strip = Image.new("RGB", (1, size))
    for y in range(size):
        t = y / max(size - 1, 1)
        strip.putpixel(
            (0, y),
            tuple(round(a + (b - a) * t) for a, b in zip(top, bottom)),
        )
    return strip.resize((size, size), Image.BILINEAR)


def diagonal_gradient(size: int, start: tuple[int, int, int], end: tuple[int, int, int]) -> Image.Image:
    """左上到右下的渐变，用于前景字形。"""
    strip = Image.new("RGB", (size * 2, 1))
    for x in range(size * 2):
        t = x / max(size * 2 - 1, 1)
        strip.putpixel((x, 0), tuple(round(a + (b - a) * t) for a, b in zip(start, end)))
    rotated = strip.resize((size, size), Image.BILINEAR).rotate(-45, expand=False, resample=Image.BICUBIC)
    return rotated


def draw_shell_glyph(mask: ImageDraw.ImageDraw, scale: float) -> None:
    """终端窗口：圆角描边 + 提示符 `>` + 光标下划线。"""
    s = scale

    def p(*values: float) -> list[float]:
        return [round(value * s) for value in values]

    stroke = round(20 * s)
    # 窗口外框
    mask.rounded_rectangle(p(112, 140, 400, 372), radius=round(30 * s), outline=255, width=stroke)
    # 提示符 `>`
    mask.line(p(168, 214, 232, 256), fill=255, width=stroke)
    mask.line(p(232, 256, 168, 298), fill=255, width=stroke)
    # 光标下划线
    mask.rounded_rectangle(p(258, 288, 344, 306), radius=round(9 * s), fill=255)


def draw_data_glyph(mask: ImageDraw.ImageDraw, scale: float) -> None:
    """数据库柱体：顶面椭圆 + 两条侧壁 + 底弧，再加两条分隔弧。"""
    s = scale

    def p(*values: float) -> list[float]:
        return [round(value * s) for value in values]

    stroke = round(20 * s)
    left, right = 136, 376
    top, bottom = 176, 340
    # 顶面
    mask.ellipse(p(left, top - 48, right, top + 48), outline=255, width=stroke)
    # 侧壁
    mask.line(p(left, top, left, bottom), fill=255, width=stroke)
    mask.line(p(right, top, right, bottom), fill=255, width=stroke)
    # 底面（椭圆下半）
    mask.arc(p(left, bottom - 48, right, bottom + 48), start=0, end=180, fill=255, width=stroke)
    # 一层分隔弧
    for center_y in (258,):
        mask.arc(
            p(left, center_y - 48, right, center_y + 48),
            start=0,
            end=180,
            fill=255,
            width=stroke,
        )


GLYPHS = {
    "navop-shell": draw_shell_glyph,
    "navop-data": draw_data_glyph,
}


def render_icon(product: str, size: int) -> Image.Image:
    """按目标尺寸渲染一张 RGBA 图标。"""
    canvas = size * SUPERSAMPLE
    scale = canvas / CANVAS

    icon = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))

    # 圆角底
    radius = round(118 * scale)
    base_mask = Image.new("L", (canvas, canvas), 0)
    ImageDraw.Draw(base_mask).rounded_rectangle(
        [round(6 * scale), round(6 * scale), canvas - round(6 * scale), canvas - round(6 * scale)],
        radius=radius,
        fill=255,
    )
    icon.paste(
        vertical_gradient(canvas, BACKGROUND_TOP, BACKGROUND_BOTTOM).convert("RGBA"),
        (0, 0),
        base_mask,
    )

    # 内描边，模拟玻璃高光
    border = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    ImageDraw.Draw(border).rounded_rectangle(
        [round(10 * scale), round(10 * scale), canvas - round(10 * scale), canvas - round(10 * scale)],
        radius=round(112 * scale),
        outline=BORDER,
        width=max(round(4 * scale), 2),
    )
    icon = Image.alpha_composite(icon, border)

    # 前景字形：先画蒙版，再用渐变填充
    glyph_mask = Image.new("L", (canvas, canvas), 0)
    mask_draw = ImageDraw.Draw(glyph_mask)
    GLYPHS[product](mask_draw, scale)
    # 圆头端点：PIL 的 line 是平头，补圆形端帽
    if product == "navop-shell":
        for center in (
            (168 * scale, 214 * scale),
            (232 * scale, 256 * scale),
            (168 * scale, 298 * scale),
        ):
            r = round(10 * scale)
            mask_draw.ellipse(
                [round(center[0] - r), round(center[1] - r), round(center[0] + r), round(center[1] + r)],
                fill=255,
            )

    glyph = diagonal_gradient(canvas, FOREGROUND_START, FOREGROUND_END).convert("RGBA")
    icon.paste(glyph, (0, 0), glyph_mask)

    return icon.resize((size, size), Image.LANCZOS)


def write_linux(product: str) -> None:
    LINUX_DIR.mkdir(parents=True, exist_ok=True)
    for size in (128, 256, 512):
        path = LINUX_DIR / f"{product}-{size}.png"
        render_icon(product, size).save(path)
        print(f"wrote {path.relative_to(PROJECT_DIR)}")


def write_windows(product: str) -> None:
    WINDOWS_DIR.mkdir(parents=True, exist_ok=True)
    path = WINDOWS_DIR / f"{product}.ico"
    sizes = [16, 32, 48, 64, 128, 256]
    largest = render_icon(product, 256)
    largest.save(path, format="ICO", sizes=[(size, size) for size in sizes])
    print(f"wrote {path.relative_to(PROJECT_DIR)}")


def write_macos(product: str) -> None:
    """用 iconutil 生成 .icns；非 macOS 环境跳过。"""
    MACOS_DIR.mkdir(parents=True, exist_ok=True)
    name = "".join(part.capitalize() for part in product.split("-"))
    target = MACOS_DIR / f"{name}.icns"
    if sys.platform != "darwin":
        print(f"skip {target.relative_to(PROJECT_DIR)}（iconutil 仅在 macOS 可用）")
        return
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "AppIcon.iconset"
        iconset.mkdir()
        for point in (16, 32, 128, 256, 512):
            for suffix, factor in (("", 1), ("@2x", 2)):
                size = point * factor
                render_icon(product, size).save(iconset / f"icon_{point}x{point}{suffix}.png")
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(target)],
            check=True,
        )
    print(f"wrote {target.relative_to(PROJECT_DIR)}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preview-dir", type=Path, help="额外输出 512px 预览图到该目录")
    args = parser.parse_args()

    for product in PRODUCTS:
        write_linux(product)
        write_windows(product)
        write_macos(product)
        if args.preview_dir:
            args.preview_dir.mkdir(parents=True, exist_ok=True)
            preview = args.preview_dir / f"{product}.png"
            render_icon(product, 512).save(preview)
            print(f"preview {preview}")


if __name__ == "__main__":
    main()
