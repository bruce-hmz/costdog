#!/usr/bin/env python3
"""单张立绘 → 宠物睡姿精灵（`pet*.png` 用的那种单帧 PNG）。

与 make-walk-sheet.py 同一套去白底算法（**从四边泛洪**，只让与边缘连通的近白像素透明），
所以白毛/白肚皮的宠物不会被抠掉；然后裁到内容包围盒、按高度缩放。
宠物页的 `.crit-sleep{height:72px;width:auto}` 只吃高度，所以这里只保证高度一致。

用法：
    make-sleep-sprite.py <输入.png> <输出.png> [--height 72] [--maxw 170] [--thresh 235]

--maxw 是保险：万一模型画了个特别宽的姿势（比如摊开躺），宽度会被夹到上限，避免睡姿
比跑图宽出一大截把铭牌挤开。
"""
import sys

import numpy as np
from PIL import Image, ImageDraw


def background_to_alpha(im, thresh=235):
    """从四边泛洪去白底：只让与边缘连通的近白像素透明，角色内部的白色（白毛/高光）保留。"""
    im = im.convert('RGB')
    w, h = im.size
    work = im.copy()
    sentinel = (255, 0, 255)
    step = 4
    for x in range(0, w, step):
        ImageDraw.floodfill(work, (x, 0), sentinel, thresh=255 - thresh)
        ImageDraw.floodfill(work, (x, h - 1), sentinel, thresh=255 - thresh)
    for y in range(0, h, step):
        ImageDraw.floodfill(work, (0, y), sentinel, thresh=255 - thresh)
        ImageDraw.floodfill(work, (w - 1, y), sentinel, thresh=255 - thresh)
    a = np.array(work)
    bg = (a[:, :, 0] == 255) & (a[:, :, 1] == 0) & (a[:, :, 2] == 255)
    return Image.fromarray(
        np.dstack([np.array(im), np.where(bg, 0, 255).astype(np.uint8)]), 'RGBA'
    )


def add_outline(im, white_px=2, shadow_px=3, shadow_alpha=70):
    """把白描边 + 柔和投影烘焙进素材（替代 CSS 的 filter: drop-shadow）。

    为什么非烘不可：走路精灵是 `div` + `background-image`，浏览器给这种元素做
    drop-shadow 时按**元素盒子**算（实测四边可见率 24%/21%/67%），于是宠物外面
    多出一个"贴纸边框"；`<img>` 才会跟着 alpha 走。烘进素材后两边一致，
    CSS 也不用再挂滤镜。
    """
    from PIL import ImageFilter
    a = im.convert('RGBA')
    alpha = a.getchannel('A')
    # 投影：向下偏移 1px 的黑色柔光
    shadow_mask = alpha.filter(ImageFilter.MaxFilter(shadow_px * 2 + 1)).point(
        lambda v: min(255, int(v * shadow_alpha / 255))
    )
    out = Image.new('RGBA', (a.size[0] + 4, a.size[1] + 6), (0, 0, 0, 0))
    dark = Image.new('RGBA', a.size, (0, 0, 0, 255))
    out.paste(dark, (2, 4), shadow_mask)
    # 白描边
    ring = alpha.filter(ImageFilter.MaxFilter(white_px * 2 + 1))
    white = Image.new('RGBA', a.size, (255, 255, 255, 255))
    out.paste(white, (2, 2), ring)
    out.paste(a, (2, 2), a)
    return out


def main():
    src, dst = sys.argv[1], sys.argv[2]
    args = sys.argv[3:]
    opt = lambda n, d: int(args[args.index(n) + 1]) if n in args else d
    height, maxw, thresh = opt('--height', 72), opt('--maxw', 170), opt('--thresh', 235)

    im = Image.open(src)
    print(f"输入 {src}: {im.size[0]}x{im.size[1]} mode={im.mode}")
    rgba = background_to_alpha(im, thresh)
    alpha = np.array(rgba)[:, :, 3]
    opaque = alpha > 24
    if opaque.sum() == 0:
        print("❌ 去白底后没有内容（参考图是不是全白/全透明？）")
        sys.exit(1)
    ys, xs = np.nonzero(opaque)
    box = (int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1)
    cropped = rgba.crop(box)
    scale = height / cropped.size[1]
    size = (max(1, round(cropped.size[0] * scale)), height)
    out = cropped.resize(size, Image.LANCZOS)
    if out.size[0] > maxw:
        print(f"⚠️ 宽度 {out.size[0]}px 超过 {maxw}px，按宽度缩放")
        out = out.resize((maxw, max(1, round(out.size[1] * maxw / out.size[0]))), Image.LANCZOS)
    if opt('--outline',1):
        out = add_outline(out, white_px=4, shadow_px=6, shadow_alpha=70)  # 3× 图按比例加粗
    out.save(dst)
    fill = 100 * opaque.mean()
    print(f"✅ 输出 {dst}: {out.size[0]}x{out.size[1]}（去白底前内容占比 {fill:.1f}%）")
    print(f"CSS 用 .crit-sleep{{height:{height}px;width:auto}} 即可，无需改尺寸")


if __name__ == '__main__':
    main()
