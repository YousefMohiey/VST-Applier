"""Build the VST-Applier icon set from the source artwork (Assets/app-source.png).

The source is a hi-res PNG of the icon (teal rounded square, microphone, waveform,
sliders) on a transparent background. This script trims it to the artwork bounds and
exports the .ico (multi-size) plus the PNG sizes used by the README and tooling.
"""
from PIL import Image
import numpy as np

ASSETS = r"C:\Users\Administrator\projects\vst-applier\VstApplier\Assets"
SOURCE = ASSETS + r"\app-source.png"


def main():
    src = Image.open(SOURCE).convert("RGBA")
    alpha = np.array(src.getchannel("A"))
    mask = (alpha >= 250).astype(np.uint8)
    ys, xs = np.where(mask)
    x0, y0, x1, y1 = xs.min(), ys.min(), xs.max(), ys.max()
    tile = src.crop((x0, y0, x1 + 1, y1 + 1))

    # normalize to a square canvas (source artwork can be a few pixels off-square)
    side = max(tile.size)
    square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    square.paste(tile, ((side - tile.width) // 2, (side - tile.height) // 2))

    square.resize((512, 512), Image.LANCZOS).save(ASSETS + r"\app.png")
    square.resize((256, 256), Image.LANCZOS).save(ASSETS + r"\app-256.png")
    square.resize((128, 128), Image.LANCZOS).save(ASSETS + r"\app-small.png")
    square.resize((48, 48), Image.LANCZOS).save(ASSETS + r"\app-48.png")
    square.resize((32, 32), Image.LANCZOS).save(ASSETS + r"\app-32.png")
    square.resize((16, 16), Image.LANCZOS).save(ASSETS + r"\app-16.png")
    square.save(
        ASSETS + r"\app.ico",
        format="ICO",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    print("icon set written from", SOURCE)


if __name__ == "__main__":
    main()
