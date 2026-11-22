"""Draw an app icon from scratch with Pillow and export .ico (16-256), PNGs and a preview sheet.

Usage:  python make_icon.py <output_folder> [name]
Replace draw_symbol() with the new design and the COLORS with the app's own colors.
Draw on the 1024 canvas; everything is downscaled with LANCZOS, so thick shapes stay readable at 16 px.
"""
import math
import os
import sys

from PIL import Image, ImageDraw

S = 1024                      # master canvas
MARGIN = 40                   # transparent margin around the rounded square (~4%)
RADIUS = 230                  # corner radius (~22%)
COLORS = {
    'top': (34, 146, 146),        # تدرّج الخلفية: فيروزي التطبيق في الأعلى
    'bottom': (18, 92, 98),       # ... فيروزي أعمق في الأسفل (دون كحلي ودون لمعة)
    'tile': (255, 248, 232, 255),    # المربعات المستقرة: كريمي
    'slot': (255, 255, 255, 46),     # مكان المربع الرابع قبل أن يستقر
    'moving': (240, 178, 74, 255),   # المربع الذي في طريقه إلى مكانه: كهرماني
    'shadow': (6, 40, 44, 80),
}
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]
PREVIEW_SIZES = [16, 24, 32, 48, 256]


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def background():
    """Rounded square with a soft vertical gradient, flat (no gloss highlight)."""
    grad = Image.new('RGB', (S, S))
    gd = ImageDraw.Draw(grad)
    for y in range(S):
        gd.line([(0, y), (S, y)], fill=lerp(COLORS['top'], COLORS['bottom'], y / (S - 1)))
    mask = Image.new('L', (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([MARGIN, MARGIN, S - MARGIN, S - MARGIN], radius=RADIUS, fill=255)
    img = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    img.paste(grad, (0, 0), mask)
    return img


def tile(size, color, shadow=None, angle=0.0):
    """مربع بزوايا مستديرة على طبقة شفافة، مع ظل خفيف، ويمكن إمالته."""
    pad = 60
    layer = Image.new('RGBA', (size + pad * 2, size + pad * 2), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    r = int(size * 0.24)
    if shadow:
        d.rounded_rectangle([pad + 8, pad + 16, pad + size + 8, pad + size + 16], radius=r, fill=shadow)
    d.rounded_rectangle([pad, pad, pad + size, pad + size], radius=r, fill=color)
    if angle:
        layer = layer.rotate(angle, resample=Image.BICUBIC)
    return layer, pad


def draw_symbol(img):
    """Array يحوّل الملفات المبعثرة إلى ترتيب: شبكة 2×2، ثلاثة مربعات استقرت في أماكنها،
    والرابع (كهرماني) مائل في طريقه إلى مكانه الفارغ. الأشكال كبيرة لتبقى واضحة بحجم 16 بكسل."""
    size, gap = 250, 64
    x0 = (S - (size * 2 + gap)) // 2
    cells = [(x0 + c * (size + gap), x0 + r * (size + gap)) for r in range(2) for c in range(2)]
    # مكان المربع الرابع (أعلى اليمين) يظهر شفافاً
    slot, pad = tile(size, COLORS['slot'])
    img.alpha_composite(slot, (cells[1][0] - pad, cells[1][1] - pad))
    for i in (0, 2, 3):
        t, pad = tile(size, COLORS['tile'], COLORS['shadow'])
        img.alpha_composite(t, (cells[i][0] - pad, cells[i][1] - pad))
    moving, pad = tile(size, COLORS['moving'], COLORS['shadow'], angle=-14)
    img.alpha_composite(moving, (cells[1][0] - pad + 34, cells[1][1] - pad - 34))


def preview_sheet(img):
    """The icon at real sizes on light and dark backgrounds: check that 16 and 32 px still read clearly."""
    pad, gap = 24, 28
    width = pad * 2 + sum(PREVIEW_SIZES) + gap * (len(PREVIEW_SIZES) - 1)
    height = pad * 2 + max(PREVIEW_SIZES) * 2 + gap
    sheet = Image.new('RGBA', (width, height), (255, 255, 255, 255))
    ImageDraw.Draw(sheet).rectangle([0, height // 2, width, height], fill=(32, 34, 38, 255))
    for row, top in enumerate((pad, pad + max(PREVIEW_SIZES) + gap)):
        x = pad
        for size in PREVIEW_SIZES:
            small = img.resize((size, size), Image.LANCZOS)
            sheet.alpha_composite(small, (x, top + (max(PREVIEW_SIZES) - size) // 2))
            x += size + gap
    return sheet


def main(out_dir, name='app'):
    os.makedirs(out_dir, exist_ok=True)
    img = background()
    draw_symbol(img)
    img.save(os.path.join(out_dir, f'{name}-1024.png'))
    img.resize((256, 256), Image.LANCZOS).save(os.path.join(out_dir, f'{name}-256.png'))
    img.save(os.path.join(out_dir, f'{name}.ico'), sizes=[(s, s) for s in ICO_SIZES])
    preview_sheet(img).save(os.path.join(out_dir, f'{name}-preview.png'))
    print('written:', ', '.join(f'{name}{suffix}' for suffix in ('.ico', '-256.png', '-1024.png', '-preview.png')))


if __name__ == '__main__':
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else 'app')
