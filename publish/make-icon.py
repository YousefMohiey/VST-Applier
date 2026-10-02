"""Generate the VST-Applier icon set v2: dark tile, dim green waveform backdrop, silver mic hero, no text."""
from PIL import Image, ImageDraw, ImageFilter

S = 1024
GREEN = (74, 222, 128)
DARK_GRAD_TOP = (30, 33, 38)
DARK_GRAD_BOTTOM = (8, 9, 11)
SILVER_HI = (240, 243, 246)
SILVER_MID = (196, 202, 210)
SILVER_LO = (128, 136, 146)


def vertical_gradient(size, top, bottom):
    grad = Image.new("RGB", (1, size), top)
    px = grad.load()
    for y in range(size):
        t = y / (size - 1)
        px[0, y] = tuple(round(top[i] + (bottom[i] - top[i]) * t) for i in range(3))
    return grad.resize((size, size))


def tri_gradient(size, top, mid, bottom):
    grad = Image.new("RGB", (1, size))
    px = grad.load()
    for y in range(size):
        t = y / (size - 1)
        if t < 0.5:
            k = t / 0.5
            px[0, y] = tuple(round(top[i] + (mid[i] - top[i]) * k) for i in range(3))
        else:
            k = (t - 0.5) / 0.5
            px[0, y] = tuple(round(mid[i] + (bottom[i] - mid[i]) * k) for i in range(3))
    return grad


def rounded_mask(size, radius):
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=255)
    return mask


def main():
    mask = rounded_mask(S, 230)

    base = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    tile = vertical_gradient(S, DARK_GRAD_TOP, DARK_GRAD_BOTTOM).convert("RGBA")
    base.paste(tile, (0, 0), mask)

    draw = ImageDraw.Draw(base)

    # --- waveform bars: dim backdrop, smooth falloff ---
    heights = [90, 150, 240, 350, 430, 350, 240, 150, 90]
    alphas = [36, 54, 88, 130, 170, 130, 88, 54, 36]
    bar_w = 24
    gap = 64
    cx, cy = S // 2, 452
    n = len(heights)
    for i, (h, a) in enumerate(zip(heights, alphas)):
        x = cx + (i - (n - 1) / 2) * gap
        draw.rounded_rectangle(
            (x - bar_w / 2, cy - h / 2, x + bar_w / 2, cy + h / 2),
            radius=bar_w // 2,
            fill=GREEN + (a,),
        )

    # --- subtle neon glow from the border only ---
    glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(glow).rounded_rectangle((30, 30, S - 31, S - 31), radius=205, outline=GREEN + (200,), width=16)
    glow = glow.filter(ImageFilter.GaussianBlur(24))
    base = Image.alpha_composite(base, glow)
    draw = ImageDraw.Draw(base)
    draw.rounded_rectangle((30, 30, S - 31, S - 31), radius=205, outline=GREEN, width=16)

    # --- microphone (hero element) ---
    # capsule with 3-stop gradient
    cap = (444, 272, 580, 532)
    cap_grad = tri_gradient(cap[3] - cap[1], SILVER_HI, SILVER_MID, SILVER_LO).convert("RGBA")
    cap_grad = cap_grad.resize((cap[2] - cap[0], cap[3] - cap[1]))
    cap_mask = Image.new("L", (cap[2] - cap[0], cap[3] - cap[1]), 0)
    ImageDraw.Draw(cap_mask).rounded_rectangle(
        (0, 0, cap[2] - cap[0] - 1, cap[3] - cap[1] - 1), radius=68, fill=255
    )
    base.paste(cap_grad, (cap[0], cap[1]), cap_mask)
    draw = ImageDraw.Draw(base)

    # soft top highlight inside capsule
    hl = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(hl).rounded_rectangle((468, 296, 556, 356), radius=30, fill=(255, 255, 255, 60))
    hl = hl.filter(ImageFilter.GaussianBlur(10))
    base = Image.alpha_composite(base, hl)
    draw = ImageDraw.Draw(base)

    # grille: 4 thin lines, soft dark
    for gy in (322, 362, 402, 442):
        draw.rounded_rectangle((466, gy, 558, gy + 10), radius=4, fill=(96, 102, 112, 185))

    # bracket arc (bottom half)
    draw.arc((380, 316, 644, 592), start=0, end=180, fill=(168, 176, 186), width=25)

    # stem + base
    draw.rounded_rectangle((495, 592, 529, 656), radius=10, fill=(184, 192, 202))
    draw.rounded_rectangle((434, 656, 590, 694), radius=19, fill=(184, 192, 202))
    # base highlight
    bh = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(bh).rounded_rectangle((444, 660, 580, 672), radius=8, fill=(255, 255, 255, 40))
    base = Image.alpha_composite(base, bh.filter(ImageFilter.GaussianBlur(4)))

    # clip everything to the tile
    final = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    final.paste(base, (0, 0), mask)
    return final


if __name__ == "__main__":
    import os

    icon = main()
    out_dir = r"C:\Users\Administrator\projects\vst-applier\VstApplier\Assets"
    os.makedirs(out_dir, exist_ok=True)
    icon.save(os.path.join(out_dir, "app.png"))
    icon.resize((256, 256), Image.LANCZOS).save(os.path.join(out_dir, "app-256.png"))
    icon.resize((128, 128), Image.LANCZOS).save(os.path.join(out_dir, "app-small.png"))
    icon.resize((48, 48), Image.LANCZOS).save(os.path.join(out_dir, "app-48.png"))
    icon.resize((32, 32), Image.LANCZOS).save(os.path.join(out_dir, "app-32.png"))
    icon.resize((16, 16), Image.LANCZOS).save(os.path.join(out_dir, "app-16.png"))
    icon.save(
        os.path.join(out_dir, "app.ico"),
        format="ICO",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    print("icon set v2 written to", out_dir)
