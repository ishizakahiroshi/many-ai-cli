"""web/src/icon.svg の要素を Pillow で描画して ICO を生成するスクリプト."""
from PIL import Image, ImageDraw, ImageFont


def hex_to_rgba(h, a=255):
    h = h.lstrip("#")
    return tuple(int(h[i:i+2], 16) for i in (0, 2, 4)) + (a,)


def render(size: int) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    s = size / 512.0

    # 背景: #111111, rx=104
    draw.rounded_rectangle(
        [0, 0, size - 1, size - 1],
        radius=max(1, round(104 * s)),
        fill=hex_to_rgba("#111111"),
    )

    # ターミナル枠: x=78,y=112, w=356,h=278, rx=34, fill=#1A1A1A, stroke=#FFD600, stroke-width=16
    sw = max(1, round(16 * s))
    x1, y1 = round(78 * s), round(112 * s)
    x2, y2 = round((78 + 356) * s), round((112 + 278) * s)
    draw.rounded_rectangle(
        [x1, y1, x2, y2],
        radius=max(1, round(34 * s)),
        fill=hex_to_rgba("#1A1A1A"),
        outline=hex_to_rgba("#FFD600"),
        width=sw,
    )

    # ウィンドウドット: (132,158), (166,158), (200,158), r=11, fill=#FFD600
    if size >= 32:
        for cx in [132, 166, 200]:
            r = max(1, round(11 * s))
            cx_s, cy_s = round(cx * s), round(158 * s)
            draw.ellipse(
                [cx_s - r, cy_s - r, cx_s + r, cy_s + r],
                fill=hex_to_rgba("#FFD600"),
            )

    # セパレータ: x1=88,y1=196, x2=424,y2=196, stroke=#3A3A3A, stroke-width=6
    lw = max(1, round(6 * s))
    draw.line(
        [(round(88 * s), round(196 * s)), (round(424 * s), round(196 * s))],
        fill=hex_to_rgba("#3A3A3A"),
        width=lw,
    )

    # CLI テキスト: x=256,y=318, font-size=116, white, anchor=middle-baseline
    fs = max(8, round(116 * s))
    font = None
    for path in [
        "C:/Windows/Fonts/consolab.ttf",   # Consolas Bold
        "C:/Windows/Fonts/cour.ttf",        # Courier New
        "C:/Windows/Fonts/arialbd.ttf",     # Arial Bold (フォールバック)
        "C:/Windows/Fonts/arial.ttf",
    ]:
        try:
            font = ImageFont.truetype(path, fs)
            break
        except OSError:
            continue

    draw.text(
        (round(256 * s), round(318 * s)),
        "CLI",
        fill=(255, 255, 255, 255),
        font=font,
        anchor="ms",  # middle-baseline
    )

    # エージェント接続ドット: (132,424), (256,424), (380,424), r=16, stroke=#FFD600
    if size >= 48:
        dot_r = max(1, round(16 * s))
        dot_sw = max(1, round(9 * s))
        for cx in [132, 256, 380]:
            cx_s, cy_s = round(cx * s), round(424 * s)
            draw.ellipse(
                [cx_s - dot_r, cy_s - dot_r, cx_s + dot_r, cy_s + dot_r],
                fill=hex_to_rgba("#111111"),
                outline=hex_to_rgba("#FFD600"),
                width=dot_sw,
            )

        # 接続線: M148 424 H240 M272 424 H364
        draw.line(
            [(round(148 * s), round(424 * s)), (round(240 * s), round(424 * s))],
            fill=hex_to_rgba("#FFD600"),
            width=max(1, round(9 * s)),
        )
        draw.line(
            [(round(272 * s), round(424 * s)), (round(364 * s), round(424 * s))],
            fill=hex_to_rgba("#FFD600"),
            width=max(1, round(9 * s)),
        )

    return img


if __name__ == "__main__":
    import os

    out = os.path.join(os.path.dirname(__file__), "..", "assets", "ai-cli-hub.ico")
    src = render(512)
    src.save(
        out,
        format="ICO",
        sizes=[(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (16, 16)],
    )
    print(f"saved: {os.path.abspath(out)}")
