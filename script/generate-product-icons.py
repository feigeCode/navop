#!/usr/bin/env python3
"""生成两个独立产品（navop-shell / navop-data）的应用图标。

品牌主图标是「N 折线 + 上升箭头」，全量产品直接用它。独立产品保留同一个 N，
只在右下角加一枚小脚标区分领域：Shell 是终端提示符，Data 是数据库柱体。

输出 Linux PNG（128/256/512）、macOS ICNS、Windows ICO（16~256）。全量产品
（navop）的图标不在这里生成，直接沿用既有设计。

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
# 品牌主图（1024，从 Navop.icns 里取出的最大尺寸），没有时退到 Linux 512。
MASTER_CANDIDATES = (
    PROJECT_DIR / "resources" / "branding" / "navop-1024.png",
    PROJECT_DIR / "resources" / "linux" / "navop-512.png",
)

# 画布先放大再缩小，避免小脚标的圆弧出现锯齿。
SUPERSAMPLE = 4

# 脚标：圆心与半径都按画布尺寸取比例，落在右下角（N 的笔画集中在中间，压不到）。
BADGE_CENTER = 0.792
BADGE_RADIUS = 0.158
BADGE_RING = 0.022
BADGE_RING_COLOR = (8, 18, 36, 255)
BADGE_START = (94, 231, 245)
BADGE_END = (63, 169, 255)
GLYPH_COLOR = (8, 20, 38, 255)

PRODUCTS = ("navop-shell", "navop-data")


def diagonal_gradient(size: int, start: tuple[int, int, int], end: tuple[int, int, int]) -> Image.Image:
    """左上到右下的渐变，用作脚标底色（按像素算，避免旋转补边留下黑角）。"""
    # 用「横条渐变 + 逐行偏移」拼出斜向渐变，纯 PIL 即可，且不会像旋转那样补黑角。
    gradient = Image.new("RGB", (size * 2, size))
    stripe = Image.new("RGB", (size * 2, 1))
    for x in range(size * 2):
        t = x / max(size * 2 - 1, 1)
        stripe.putpixel((x, 0), tuple(round(a + (b - a) * t) for a, b in zip(start, end)))
    stripe = stripe.resize((size * 2, size), Image.BILINEAR)
    for y in range(size):
        gradient.paste(stripe.crop((y, 0, y + size, size)).crop((0, 0, size, 1)), (0, y))
    return gradient.crop((0, 0, size, size))


def draw_shell_badge(mask: ImageDraw.ImageDraw, center: tuple[float, float], radius: float) -> None:
    """终端：提示符 `>` + 光标下划线。"""
    cx, cy = center
    stroke = max(round(radius * 0.22), 2)

    def p(dx: float, dy: float) -> tuple[int, int]:
        return (round(cx + dx * radius), round(cy + dy * radius))

    # `>`
    mask.line([*p(-0.44, -0.34), *p(0.04, 0.0)], fill=255, width=stroke)
    mask.line([*p(0.04, 0.0), *p(-0.44, 0.34)], fill=255, width=stroke)
    # 圆头端点
    for dx, dy in ((-0.44, -0.34), (0.04, 0.0), (-0.44, 0.34)):
        x, y = p(dx, dy)
        half = stroke / 2
        mask.ellipse([x - half, y - half, x + half, y + half], fill=255)
    # 下划线
    mask.rounded_rectangle(
        [*p(0.20, 0.20), *p(0.58, 0.34)],
        radius=round(radius * 0.08),
        fill=255,
    )


def draw_data_badge(mask: ImageDraw.ImageDraw, center: tuple[float, float], radius: float) -> None:
    """数据库：顶面椭圆 + 一条分隔弧 + 底弧。"""
    cx, cy = center
    stroke = max(round(radius * 0.155), 2)
    half_width = radius * 0.60
    half_height = radius * 0.18
    top = cy - radius * 0.34
    bottom = cy + radius * 0.34

    def box(y: float) -> list[int]:
        return [
            round(cx - half_width),
            round(y - half_height),
            round(cx + half_width),
            round(y + half_height),
        ]

    # 顶面
    mask.ellipse(box(top), outline=255, width=stroke)
    # 侧壁
    for x in (round(cx - half_width), round(cx + half_width)):
        mask.line([x, round(top), x, round(bottom)], fill=255, width=stroke)
    # 底面
    mask.arc(box(bottom), start=0, end=180, fill=255, width=stroke)
    # 分隔弧
    mask.arc(box(cy + radius * 0.01), start=0, end=180, fill=255, width=stroke)


BADGES = {
    "navop-shell": draw_shell_badge,
    "navop-data": draw_data_badge,
}


def load_master() -> Image.Image:
    for candidate in MASTER_CANDIDATES:
        if candidate.exists():
            return Image.open(candidate).convert("RGBA")
    raise SystemExit("找不到品牌主图：期望 resources/branding/navop-1024.png")


def render_icon(master: Image.Image, product: str, size: int) -> Image.Image:
    """主图缩放到目标尺寸，叠上产品脚标。"""
    work = size * SUPERSAMPLE
    icon = master.resize((work, work), Image.LANCZOS)

    center = work * BADGE_CENTER
    radius = work * BADGE_RADIUS

    # 外圈深色描边，把脚标和主图背景分开。
    ring = Image.new("RGBA", (work, work), (0, 0, 0, 0))
    ring_radius = radius + work * BADGE_RING
    ImageDraw.Draw(ring).ellipse(
        [
            round(center - ring_radius),
            round(center - ring_radius),
            round(center + ring_radius),
            round(center + ring_radius),
        ],
        fill=BADGE_RING_COLOR,
    )
    icon = Image.alpha_composite(icon, ring)

    # 脚标底色
    disc = Image.new("L", (work, work), 0)
    ImageDraw.Draw(disc).ellipse(
        [
            round(center - radius),
            round(center - radius),
            round(center + radius),
            round(center + radius),
        ],
        fill=255,
    )
    icon.paste(diagonal_gradient(work, BADGE_START, BADGE_END).convert("RGBA"), (0, 0), disc)

    # 脚标字形
    glyph_mask = Image.new("L", (work, work), 0)
    BADGES[product](ImageDraw.Draw(glyph_mask), (center, center), radius)
    icon.paste(Image.new("RGBA", (work, work), GLYPH_COLOR), (0, 0), glyph_mask)

    return icon.resize((size, size), Image.LANCZOS)


def write_linux(master: Image.Image, product: str) -> None:
    LINUX_DIR.mkdir(parents=True, exist_ok=True)
    for size in (128, 256, 512):
        path = LINUX_DIR / f"{product}-{size}.png"
        render_icon(master, product, size).save(path)
        print(f"wrote {path.relative_to(PROJECT_DIR)}")


def write_windows(master: Image.Image, product: str) -> None:
    WINDOWS_DIR.mkdir(parents=True, exist_ok=True)
    path = WINDOWS_DIR / f"{product}.ico"
    sizes = [16, 32, 48, 64, 128, 256]
    render_icon(master, product, 256).save(
        path, format="ICO", sizes=[(size, size) for size in sizes]
    )
    print(f"wrote {path.relative_to(PROJECT_DIR)}")


def write_macos(master: Image.Image, product: str) -> None:
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
                render_icon(master, product, point * factor).save(
                    iconset / f"icon_{point}x{point}{suffix}.png"
                )
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(target)],
            check=True,
        )
    print(f"wrote {target.relative_to(PROJECT_DIR)}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preview-dir", type=Path, help="额外输出 512px 预览图到该目录")
    args = parser.parse_args()

    master = load_master()
    for product in PRODUCTS:
        write_linux(master, product)
        write_windows(master, product)
        write_macos(master, product)
        if args.preview_dir:
            args.preview_dir.mkdir(parents=True, exist_ok=True)
            preview = args.preview_dir / f"{product}.png"
            render_icon(master, product, 512).save(preview)
            print(f"preview {preview}")


if __name__ == "__main__":
    main()
