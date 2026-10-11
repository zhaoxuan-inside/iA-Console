#!/usr/bin/env python3
"""生成 iA Console 应用图标（256x256 PNG，零依赖：zlib+struct 手写 PNG 编码）。
设计语言：夜色底 + accent 色竖条（提案卡左侧色条的具象化）+ fg 色光标块。
用法：gen-icon.py <输出路径.png>
"""
import struct
import sys
import zlib

W = H = 256

# 主题色（_04/03 夜色令牌）
BG = (0x1B, 0x1D, 0x22)
ACCENT = (0x6E, 0x9B, 0xFF)
FG = (0xD6, 0xD8, 0xDD)
FG25 = (0x3A, 0x3E, 0x46)


def inside_rounded(x, y, radius=40):
    """圆角矩形内判定（整图即矩形，四角削圆）。"""
    corners = [
        (radius, radius),
        (W - radius, radius),
        (radius, H - radius),
        (W - radius, H - radius),
    ]
    for cx, cy in corners:
        in_corner_zone = (x < cx or x > W - radius) and (y < cy or y > H - radius)
        # 仅四角区域内需要距离判定
        near_corner_x = x < cx if cx < W // 2 else x > W - radius
        near_corner_y = y < cy if cy < H // 2 else y > H - radius
        if near_corner_x and near_corner_y:
            dx, dy = x - cx, y - cy
            if dx * dx + dy * dy > radius * radius:
                return False
    return True


def pixel(x, y):
    if not inside_rounded(x, y):
        return (0, 0, 0, 0)  # 透明
    # 左侧 accent 色条（x 32~56，y 64~192）
    if 32 <= x < 56 and 64 <= y < 192:
        return (*ACCENT, 255)
    # 提案行三行：模拟文本行（fg 色圆角短条）
    for (row_y, width) in [(72, 120), (116, 160), (160, 96)]:
        if row_y <= y < row_y + 20 and 76 <= x < 76 + width:
            return (*FG, 230)
    # 底部光标块（accent）
    if 176 <= y < 196 and 76 <= x < 100:
        return (*ACCENT, 255)
    # 其余：背景（稀疏网格点用 fg25 点缀）
    if x % 32 == 0 and y % 32 == 0:
        return (*FG25, 255)
    return (*BG, 255)


def build_png(path):
    raw = b""
    for y in range(H):
        raw += b"\x00"  # filter: none
        for x in range(W):
            raw += bytes(pixel(x, y))

    def chunk(tag, data):
        c = tag + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0)  # 8bit RGBA
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )
    with open(path, "wb") as f:
        f.write(png)
    print(f"图标已生成：{path}")


if __name__ == "__main__":
    build_png(sys.argv[1])
