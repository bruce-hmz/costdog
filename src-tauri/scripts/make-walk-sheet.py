#!/usr/bin/env python3
"""ChatGPT 横排行走图 → pet.html 雪碧图（与 walk-sheet.png 同格式：内容在 y 0..71，等距帧）

用法: python3 make-walk-sheet.py <in.png> <out.png> [--height 72] [--gap 3]
前置: 用 scripts/chatgpt-gen-image.mjs 生成横排 8 帧原图（成品 2172×724）。

关键点：
  * 优先按模型自己的等分布局切帧（保留原图对齐）——逐帧 bbox 居中会因腿部摆幅不同
    导致身体左右抖动；只有布局明显不均（段宽/间距方差超阈值）才退回 bbox + 上半身质心对齐。
  * 去白底用"从四边泛洪"而不是全局白阈值：白毛角色（灰白兔、白高光）不会被抠掉。
  * 输出的 background-size / 关键帧位移会打印出来，直接抄进 pet.html 即可。
"""
import sys
from PIL import Image, ImageDraw
import numpy as np

def background_to_alpha(im, thresh=235):
    """从四边泛洪去白底：只让与边缘连通的近白像素透明，角色内部的白色（白毛/高光）保留"""
    im = im.convert('RGB'); w, h = im.size
    work = im.copy(); sentinel = (255, 0, 255)
    step = 4
    for x in range(0, w, step):
        ImageDraw.floodfill(work, (x, 0), sentinel, thresh=255-thresh)
        ImageDraw.floodfill(work, (x, h-1), sentinel, thresh=255-thresh)
    for y in range(0, h, step):
        ImageDraw.floodfill(work, (0, y), sentinel, thresh=255-thresh)
        ImageDraw.floodfill(work, (w-1, y), sentinel, thresh=255-thresh)
    a = np.array(work)
    bg = (a[:,:,0]==255)&(a[:,:,1]==0)&(a[:,:,2]==255)
    return Image.fromarray(np.dstack([np.array(im), np.where(bg,0,255).astype(np.uint8)]), 'RGBA')

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


def ink_runs(ink, min_w=8, min_gap=2):
    cols = ink.any(axis=0); runs=[]; s=None; gap=0
    for i,v in enumerate(cols):
        if v:
            if s is None: s=i
            gap=0
        else:
            if s is not None:
                gap+=1
                if gap>=min_gap:
                    if i-gap-s+1>min_w: runs.append((s,i-gap))
                    s=None
    if s is not None and len(cols)-s>min_w: runs.append((s,len(cols)-1))
    return runs

def main():
    src, dst = sys.argv[1], sys.argv[2]
    args = sys.argv[3:]
    opt = lambda n,d: int(args[args.index(n)+1]) if n in args else d
    CH, GAP = opt('--height',72), opt('--gap',3)
    im = Image.open(src); print(f"输入 {src}: {im.size[0]}x{im.size[1]} mode={im.mode}")
    rgba = background_to_alpha(im); a = np.array(rgba); ink = a[:,:,3] > 24
    print(f"去白底后墨迹占比 {100*ink.mean():.1f}%")
    runs = ink_runs(ink)
    print(f"横向图形段 {len(runs)} 个: {runs}")
    N = len(runs)
    if N < 2: print("❌ 只检测到一个图形，无法做成走路循环"); sys.exit(1)
    centers = [ (x0+x1)/2 for x0,x1 in runs ]
    spacings = [centers[i+1]-centers[i] for i in range(N-1)]
    widths = [x1-x0+1 for x0,x1 in runs]
    print(f"段宽 {widths}\n段心距 {[round(s,1) for s in spacings]}")
    even = (max(spacings)-min(spacings)) < 0.15*np.mean(spacings) and (max(widths)-min(widths)) < 0.35*np.mean(widths)
    W = im.size[0]
    if even:
        print("→ 采用等分布局切帧（保留原图对齐）")
        pitch_src = W/N
        crops = [ rgba.crop((round(i*pitch_src),0,round((i+1)*pitch_src),im.size[1])) for i in range(N) ]
        # 每帧再按各自墨迹裁掉上下空白（横向保持等分，避免左右抖动）
        fixed=[]
        for c in crops:
            bb=c.getbbox()
            fixed.append(c.crop((0,bb[1],c.size[0],bb[3])))
        ytop = [c.getbbox()[1] for c in crops]
        print("各帧内容顶部 y:", ytop)
        frames=[c.resize((max(1,round(c.size[0]*CH/c.size[1])), CH), Image.LANCZOS) for c in fixed]
    else:
        print("→ 布局不均，退回 bbox 裁剪 + 上半身质心对齐")
        frames=[]; anchors=[]
        for (x0,x1) in runs:
            sub = rgba.crop((x0,0,x1+1,im.size[1])); bb=sub.getbbox(); sub=sub.crop(bb)
            top = np.array(sub)[:max(1,int(sub.size[1]*0.45)),:,3] > 24
            ax = float(np.nonzero(top)[1].mean()) if top.any() else sub.size[0]/2
            anchors.append(ax); frames.append(sub)
        print("质心 x:", [round(x,1) for x in anchors])
    if not even:
        scaled=[f.resize((max(1,round(f.size[0]*CH/f.size[1])), CH), Image.LANCZOS) for f in frames]
        anchors_s=[a*CH/f.size[1] for a,f in zip(anchors,frames)]
        pw = max(f.size[0] for f in scaled)
        pitch = pw + GAP
        sheet = Image.new('RGBA',(pitch*len(scaled),CH),(0,0,0,0))
        for i,f in enumerate(scaled):
            x = round(i*pitch + pw/2 - anchors_s[i])
            x = max(i*pitch, min(x, i*pitch+pw-f.size[0]))
            sheet.alpha_composite(f,(x,0))
        frames_out = scaled
    else:
        pw = max(f.size[0] for f in frames); pitch = pw + GAP
        sheet = Image.new('RGBA',(pitch*len(frames),CH),(0,0,0,0))
        for i,f in enumerate(frames): sheet.alpha_composite(f,(i*pitch+(pw-f.size[0])//2,0))
        frames_out = frames
    if opt('--outline',1):
        sheet = add_outline(sheet)
    sheet.save(dst)
    print(f"✅ 输出 {dst}: {sheet.size[0]}x{sheet.size[1]}  {len(frames_out)} 帧 帧宽={[f.size[0] for f in frames_out]} pitch={pitch}")
    pw2=max(f.size[0] for f in frames_out)
    def norm(f):
        c=Image.new('RGBA',(pw2,CH),(0,0,0,0)); c.alpha_composite(f,(0,0)); return np.array(c).astype(np.float32)
    arr=[norm(f) for f in frames_out]
    inks=[int((np.array(f)[:,:,3]>24).sum()) for f in frames_out]
    inter=[float(np.abs(arr[i]-arr[j]).mean()) for i in range(len(arr)) for j in range(i+1,len(arr))]
    # 相邻帧差异（走路循环的相邻帧最像，首末帧应接近以形成闭环）
    adj=[float(np.abs(arr[i]-arr[i+1]).mean()) for i in range(len(arr)-1)]
    print("相邻帧差异:", [round(x,1) for x in adj], " 首末帧差异:", round(float(np.abs(arr[0]-arr[-1]).mean()),1))
    print("逐帧墨迹:", inks)
    print(f"帧间差异: min={min(inter):.1f} mean={np.mean(inter):.1f} max={max(inter):.1f}")
    print(f"CSS: #x-run.sheet{{width:{max(f.size[0] for f in frames_out)}px;height:{CH}px;background-size:{sheet.size[0]}px {CH}px}} "
          f"@keyframes xw{{from{{background-position:0 0}}to{{background-position:-{sheet.size[0]}px 0}}}} steps({len(frames_out)})")

    # ── 步幅（stride）：脚（最低墨迹）在一个循环里相对身体前后移动多少像素 ──
    # 这个数直接决定"脚不打滑"的地面速度上限 = 步幅 / 循环时长。
    # 步幅太小是"跑起来像在冰上滑"的根因：狗 8.6px / 猫 10.0px / 兔 5.8px（旧素材）
    # 配 1s 循环只能走 <10px/s，而当时身体跑 70px/s → 打滑 8 倍。
    feet = []
    for f in frames_out:
        a = np.array(f)
        ys, xs = np.nonzero(a[:, :, 3] > 40)
        if len(ys) == 0:
            feet.append(None)
            continue
        low = ys.max()
        feet.append(float(xs[ys >= low - 3].mean()))
    valid = [x for x in feet if x is not None]
    stride = (max(valid) - min(valid)) if valid else 0.0
    body_w = max(f.size[0] for f in frames_out)
    print("逐帧脚位(最低3行重心x):", [None if x is None else round(x, 1) for x in feet])
    print(f"步幅 stride = {stride:.1f}px  （{stride / body_w * 100:.0f}% 身宽；<20% 会明显打滑）")
    for cycle_ms in (400, 600, 800):
        print(f"  循环 {cycle_ms}ms → 脚不打滑的地面速度 ≈ {stride / (cycle_ms / 1000):.1f} px/s"
              f"（≈{stride / (cycle_ms / 1000) * 0.110:.2f}px / 110ms tick）")

main()
