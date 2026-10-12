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
    'top': (38, 150, 150),        # تدرّج الخلفية: فيروزي في الأعلى
    'bottom': (20, 52, 92),       # ... كحلي في الأسفل
    'back': (240, 178, 74, 255),  # المجلد الخلفي: كهرماني
    'middle': (250, 206, 120, 255),  # المجلد الأوسط: كهرماني فاتح
    'front': (255, 248, 232, 255),   # المجلد الأمامي: كريمي
    'shadow': (10, 30, 50, 70),
}
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]
PREVIEW_SIZES = [16, 24, 32, 48, 256]


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def background():
    """Rounded square with a vertical gradient and a soft top highlight."""
    grad = Image.new('RGB', (S, S))
    gd = ImageDraw.Draw(grad)
    for y in range(S):
        gd.line([(0, y), (S, y)], fill=lerp(COLORS['top'], COLORS['bottom'], y / (S - 1)))
    mask = Image.new('L', (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([MARGIN, MARGIN, S - MARGIN, S - MARGIN], radius=RADIUS, fill=255)
    img = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    img.paste(grad, (0, 0), mask)
    highlight = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(highlight).ellipse([-200, -620, S + 200, 420], fill=(255, 255, 255, 38))
    return Image.alpha_composite(img, Image.composite(highlight, Image.new('RGBA', (S, S)), mask))


def draw_folder(d, x, y, w, h, color, shadow):
    """مجلد: لسان صغير أعلى اليسار + جسم بزوايا مستديرة، مع ظل خفيف تحته."""
    tab_w, tab_h, r = int(w * 0.38), int(h * 0.22), int(h * 0.16)
    d.rounded_rectangle([x + 10, y + 18, x + w + 10, y + h + 18], radius=r, fill=shadow)
    d.rounded_rectangle([x, y - tab_h + r, x + tab_w, y + r * 2], radius=r, fill=color)
    d.rounded_rectangle([x, y, x + w, y + h], radius=r, fill=color)


def draw_symbol(img):
    """Array يرتّب الملفات المبعثرة في مجلدات: ثلاثة مجلدات مصفوفة فوق بعضها بترتيب منتظم.
    الخطوط والأشكال كبيرة حتى تبقى واضحة بحجم 16 بكسل."""
    layer = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    w, h = 600, 300
    x0 = (S - w) // 2
    # من الخلف إلى الأمام، كل مجلد أسفل السابق قليلاً
    for i, key in enumerate(('back', 'middle', 'front')):
        y = 250 + i * 130
        draw_folder(d, x0 + (2 - i) * 0 , y, w, h, COLORS[key], COLORS['shadow'])
    # خط ملصق على المجلد الأمامي: يوحي بأن المحتوى مرتّب ومسمّى
    fy = 250 + 2 * 130
    d.rounded_rectangle([x0 + 90, fy + 120, x0 + w - 90, fy + 168], radius=24, fill=(20, 52, 92, 255))
    img.alpha_composite(layer)


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
