#!/usr/bin/env python3
"""走路雪碧图验收：判断一张（未切的）生图能不能当 walk cycle 用。

为什么要它：模型很容易画出"八帧都在原地搓腿"的图 —— 每帧三四只脚全贴地，
没有摆动相（某只脚明显离地）。这种图切出来再配上速度也是"滑"，不是"走"。
肉眼看八张缩略图不可靠，所以把判据量化：

  1. 摆动相：至少 2 帧里"触地脚簇" ≤ 2（说明有脚抬起来了）
  2. 抬脚高度：抬起的那只脚，爪底距画面底线 ≥ 6px
  3. 步幅：脚（最低墨迹重心）在循环内前后移动 ≥ 12% 身宽
  4. 身高稳定：各帧"躯干中段背线"极差 ≤ 4px（起伏由腿体现，不是整体上下跳）
  5. 质量一致：各帧墨迹面积极差 ≤ 25%（不要一帧胖一帧瘦）
用法：check-walk-sheet.py <原图.png> [--frames 8] [--minlift 6] [--json]
"""
import json
import sys

import numpy as np
from PIL import Image, ImageDraw


def background_to_alpha(im, thresh=235):
    """与 make-walk-sheet.py 同一套四边泛洪去白底。"""
    im = im.convert('RGB')
    w, h = im.size
    work = im.copy()
    sentinel = (255, 0, 255)
    for x in range(0, w, 4):
        ImageDraw.floodfill(work, (x, 0), sentinel, thresh=255 - thresh)
        ImageDraw.floodfill(work, (x, h - 1), sentinel, thresh=255 - thresh)
    for y in range(0, h, 4):
        ImageDraw.floodfill(work, (0, y), sentinel, thresh=255 - thresh)
        ImageDraw.floodfill(work, (w - 1, y), sentinel, thresh=255 - thresh)
    a = np.array(work)
    bg = (a[:, :, 0] == 255) & (a[:, :, 1] == 0) & (a[:, :, 2] == 255)
    return Image.fromarray(np.dstack([np.array(im), np.where(bg, 0, 255).astype(np.uint8)]), 'RGBA')


def ink_runs(cols, min_gap=6):
    runs = []
    s = None
    gap = 0
    for i, v in enumerate(cols):
        if v:
            if s is None:
                s = i
            gap = 0
        else:
            if s is not None:
                gap += 1
                if gap >= min_gap:
                    runs.append((s, i - gap))
                    s = None
    if s is not None:
        runs.append((s, len(cols) - 1))
    return runs


def analyse(path, frames=8, min_lift=6):
    im = background_to_alpha(Image.open(path))
    a = np.array(im)
    ink = a[:, :, 3] > 40
    runs = ink_runs(ink.any(axis=0), min_gap=6)
    if len(runs) != frames:
        # 粘连/多画：退回等分
        cols = np.zeros(im.size[0], bool)
        cols[ink.any(axis=0)] = True
        step = im.size[0] / frames
        runs = [(int(i * step), int((i + 1) * step) - 1) for i in range(frames)]
    report = {'segments': len(runs), 'frames': []}
    for i, (x0, x1) in enumerate(runs[:frames]):
        cell = ink[:, x0:x1 + 1]
        ys, xs = np.nonzero(cell)
        if len(ys) == 0:
            report['frames'].append(None)
            continue
        bottom = int(ys.max())
        # 触地脚簇：底部 3 行内的墨迹列簇
        low = cell[max(0, bottom - 2):bottom + 1, :].any(axis=0)
        clusters = 0
        prev = False
        for c in low:
            if c and not prev:
                clusters += 1
            prev = c
        # 抬脚高度：最低墨迹之上，看"离地"的局部最低点（爪底与底线之间的空隙）
        # 取每一列的最低墨迹 y，找到明显高于底线的列（说明那只脚抬起来了）
        col_low = np.full(cell.shape[1], bottom)
        for c in range(cell.shape[1]):
            rows = np.nonzero(cell[:, c])[0]
            if len(rows):
                col_low[c] = rows.max()
        lifted = int((col_low <= bottom - min_lift).sum())
        # 躯干背线：中段列的最上一行墨迹
        bx0, bx1 = xs.min(), xs.max()
        m0, m1 = int(bx0 + (bx1 - bx0) * 0.35), int(bx0 + (bx1 - bx0) * 0.65)
        mid = cell[:, m0:m1 + 1]
        rows = np.nonzero(mid.any(axis=1))[0]
        report['frames'].append({
            'x0': x0, 'x1': x1, 'w': x1 - x0 + 1,
            'mass': int(cell.sum()),
            'contacts': clusters,
            'lifted_cols': lifted,
            'back_y': int(rows.min()) if len(rows) else None,
            'bottom': bottom,
            'foot_x': float(np.nonzero(cell[bottom - 2:bottom + 1, :])[1].mean()) if clusters else None,
        })
    valid = [f for f in report['frames'] if f]
    if not valid:
        report['verdict'] = 'FAIL: 没检测到内容'
        return report
    swing = [f for f in valid if f['contacts'] <= 2]
    lift_frames = [f for f in valid if f['lifted_cols'] >= 3]
    backs = [f['back_y'] for f in valid if f['back_y'] is not None]
    masses = [f['mass'] for f in valid]
    foot_xs = [f['foot_x'] for f in valid if f['foot_x'] is not None]
    width = float(np.mean([f['w'] for f in valid]))
    stride = (max(foot_xs) - min(foot_xs)) if foot_xs else 0.0
    report['summary'] = {
        'swing_frames': len(swing),
        'lift_frames': len(lift_frames),
        'contacts': [f['contacts'] for f in valid],
        'max_lifted_cols': max(f['lifted_cols'] for f in valid),
        'back_spread': (max(backs) - min(backs)) if backs else None,
        'mass_spread_pct': round(100 * (max(masses) - min(masses)) / max(1, np.mean(masses)), 1),
        'stride_px': round(stride, 1),
        'stride_pct_of_width': round(100 * stride / max(1, width), 1),
    }
    s = report['summary']
    checks = [
        ('swing phase (≥2 frames with ≤2 contacts)', s['swing_frames'] >= 2),
        (f'foot lift (≥3 columns lifted ≥{min_lift}px)', s['lift_frames'] >= 1),
        ('stride ≥12% body width', s['stride_pct_of_width'] >= 12),
        ('body height stable (back spread ≤4px)', (s['back_spread'] is not None and s['back_spread'] <= 4)),
        ('mass consistent (≤25%)', s['mass_spread_pct'] <= 25),
    ]
    report['checks'] = [{'name': n, 'pass': bool(p)} for n, p in checks]
    report['verdict'] = 'PASS' if all(p for _, p in checks) else 'FAIL'
    return report


def main():
    path = sys.argv[1]
    args = sys.argv[2:]
    opt = lambda n, d: int(args[args.index(n) + 1]) if n in args else d
    r = analyse(path, frames=opt('--frames', 8), min_lift=opt('--minlift', 6))
    if '--json' in args:
        print(json.dumps(r, ensure_ascii=False))
        return
    print(f"输入 {path}")
    s = r.get('summary')
    if not s:
        print(r.get('verdict'))
        sys.exit(1)
    print(f"  触地脚簇/帧: {s['contacts']}   (走路的摆动相应出现 ≤2)")
    print(f"  有脚离地的帧: {s['lift_frames']} / {len(r['frames'])}   最大离地列数 {s['max_lifted_cols']}")
    print(f"  步幅 {s['stride_px']}px = {s['stride_pct_of_width']}% 身宽")
    print(f"  背线极差 {s['back_spread']}px   墨迹面积极差 {s['mass_spread_pct']}%")
    for c in r['checks']:
        print(f"  [{'✓' if c['pass'] else '✗'}] {c['name']}")
    print(f"  结论: {r['verdict']}")
    sys.exit(0 if r['verdict'] == 'PASS' else 1)


if __name__ == '__main__':
    main()
