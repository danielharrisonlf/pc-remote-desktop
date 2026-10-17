"""Render the shared SVG identity; no platform/package/signing settings are edited.

python -m pip install -r brand/requirements.txt
python scripts/generate-icons.py --android ../pc-remote-android
python scripts/generate-icons.py --android ../pc-remote-android --check
"""
from __future__ import annotations

import argparse
import io
import math
from pathlib import Path
import struct
import xml.etree.ElementTree as ET

from PIL import Image, ImageDraw, ImageFont
import resvg_py

ROOT = Path(__file__).resolve().parents[1]
BG = "#0F141C"
NS = "http://www.w3.org/2000/svg"
ANDROID_NS = "http://schemas.android.com/apk/res/android"
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)


def paths() -> list[dict[str, str]]:
    return [dict(node.attrib) for node in ET.parse(ROOT / "brand/symbol.svg").getroot()
            if node.tag == f"{{{NS}}}path"]


def svg(*, background: str | None = None, zoom: bool = False,
        monochrome: bool = False, round_icon: bool = False) -> str:
    backdrop = ""
    if background:
        backdrop = (f'<circle cx="54" cy="54" r="54" fill="{background}"/>' if round_icon else
                    f'<rect width="108" height="108" rx="22" fill="{background}"/>')
    content = ""
    for path in paths():
        attrs = path | ({"fill": "#FFFFFF"} if monochrome else {})
        content += "<path " + " ".join(f'{key}="{value}"' for key, value in attrs.items()) + "/>"
    if zoom:
        content = '<g transform="translate(-18 -18) scale(1.333333333333)">' + content + "</g>"
    return (f'<svg xmlns="{NS}" width="108" height="108" viewBox="0 0 108 108">'
            '<title>PC Remote</title>' + backdrop + content + "</svg>\n")


def render(source: str, size: int) -> bytes:
    # Independent vector rendering at every output size, including tiny ICO layers.
    return resvg_py.svg_to_bytes(svg_string=source, width=size, height=size)


def vector(*, monochrome: bool = False, notification: bool = False) -> bytes:
    size = 24 if notification else 108
    lines = [f'<vector xmlns:android="{ANDROID_NS}" android:width="{size}dp" '
             f'android:height="{size}dp" android:viewportWidth="108" android:viewportHeight="108">']
    if notification:
        lines.append('  <group android:scaleX="1.5" android:scaleY="1.5" '
                     'android:pivotX="54" android:pivotY="54">')
    for path in paths():
        color = "#FFFFFF" if monochrome else path["fill"]
        rule = ' android:fillType="evenOdd"' if path.get("fill-rule") else ""
        lines.append(f'  <path android:fillColor="{color}"{rule} android:pathData="{path["d"]}"/>')
    if notification:
        lines.append("  </group>")
    lines.append("</vector>")
    return ("\n".join(lines) + "\n").encode()


def ico(source: str) -> bytes:
    layers = [render(source, size) for size in ICO_SIZES]
    offset = 6 + 16 * len(layers)
    entries = []
    for size, layer in zip(ICO_SIZES, layers):
        entries.append(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32,
                                   len(layer), offset))
        offset += len(layer)
    return struct.pack("<HHH", 0, 1, len(layers)) + b"".join(entries + layers)


def preview(source: str) -> bytes:
    board = Image.new("RGB", (1440, 940), "#161D29")
    draw = ImageDraw.Draw(board)
    font = ImageFont.load_default(size=20)
    small = ImageFont.load_default(size=15)
    title = ImageFont.load_default(size=34)
    draw.text((48, 32), "PC Remote / Linked frame", fill="#F8FAFC", font=title)
    draw.text((48, 83), "Obsidian 0F141C   /   Ice F8FAFC   /   Signal 00E5FF", fill="#A8B4C4", font=font)
    master = Image.open(io.BytesIO(render(source, 340))).convert("RGBA")
    board.paste(master, (64, 147), master)
    draw.text((64, 510), "DESKTOP MASTER", fill="#A8B4C4", font=small)
    x = 480
    for size in (16, 24, 32, 48, 64, 128, 256):
        im = Image.open(io.BytesIO(render(source, size))).convert("RGBA")
        board.paste(im, (x, 190 + (256 - size) // 2), im)
        draw.text((x, 470), f"{size}px", fill="#A8B4C4", font=small)
        x += max(size + 24, 62)
    draw.text((48, 583), "ANDROID / CIRCLE, SQUIRCLE, ROUNDED SQUARE, THEMED, SAFE ZONE", fill="#A8B4C4", font=font)
    fg = Image.open(io.BytesIO(render(svg(), 300))).convert("RGBA")
    bg = Image.new("RGBA", (300, 300), BG)
    full = Image.alpha_composite(bg, fg)
    # Android masks are applied to the 72dp viewport, not the full 108dp layer.
    crop = full.crop((50, 50, 250, 250)).resize((180, 180), Image.Resampling.LANCZOS)
    mono = Image.open(io.BytesIO(render(svg(monochrome=True), 300))).convert("RGBA")
    mono.putdata([(188, 218, 240, p[3]) for p in mono.get_flattened_data()])
    themed = Image.alpha_composite(Image.new("RGBA", (300, 300), "#263B4A"), mono)
    themed = themed.crop((50, 50, 250, 250)).resize((180, 180), Image.Resampling.LANCZOS)
    for index, radius in enumerate((90, 56, 36, 90)):
        mask = Image.new("L", (720, 720))
        ImageDraw.Draw(mask).rounded_rectangle((0, 0, 719, 719), radius=radius * 4, fill=255)
        mask = mask.resize((180, 180), Image.Resampling.LANCZOS)
        board.paste(themed if index == 3 else crop, (64 + index * 245, 650), mask)
    safe = full.resize((180, 180), Image.Resampling.LANCZOS)
    board.paste(safe, (1044, 650))
    draw.ellipse((1079, 685, 1189, 795), outline="#FFB55B", width=1)
    draw.rectangle((1079, 685, 1189, 795), outline="#667386", width=1)
    draw.text((1044, 853), "108dp / central 66dp", fill="#A8B4C4", font=small)
    output = io.BytesIO()
    board.save(output, format="PNG")
    return output.getvalue()


def validate_safe_zone() -> None:
    im = Image.open(io.BytesIO(render(svg(), 1080))).convert("RGBA")
    bounds = im.getbbox()
    assert bounds and all(210 <= n <= 870 for n in bounds), bounds
    # Also contain meaningful pixels in the central 66dp diameter circle.
    radius = max(math.hypot((x + .5) / 10 - 54, (y + .5) / 10 - 54)
                 for y in range(bounds[1], bounds[3]) for x in range(bounds[0], bounds[2])
                 if im.getpixel((x, y))[3] >= 128)
    assert radius <= 33, f"Foreground extends outside safe circle: {radius:.2f}dp"
    print(f"Adaptive foreground: bounds {tuple(n / 10 for n in bounds)}dp; radius {radius:.2f}/33dp")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--android", type=Path, help="Native Android repo; explicitly selected, never auto-discovered")
    parser.add_argument("--check", action="store_true", help="Compare generated assets without modifying files")
    args = parser.parse_args()
    validate_safe_zone()
    desktop = svg(background=BG, zoom=True)
    outputs = {ROOT / "brand/icon.svg": desktop.encode(),
               ROOT / "src/assets/brand.svg": desktop.encode(),
               ROOT / "public/icon.svg": desktop.encode(),
               ROOT / "brand/preview.png": preview(desktop),
               ROOT / "src-tauri/icons/icon.ico": ico(desktop)}
    for name, size in (("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256),
                       ("icon.png", 512), ("icon-1024.png", 1024)):
        outputs[ROOT / "src-tauri/icons" / name] = render(desktop, size)
    if args.android:
        res = args.android.resolve() / "android/app/src/main/res"
        assert (res.parent / "AndroidManifest.xml").is_file(), f"Not a native Android resource root: {res}"
        outputs[res / "drawable/ic_launcher_foreground.xml"] = vector()
        outputs[res / "drawable/ic_launcher_monochrome.xml"] = vector(monochrome=True)
        outputs[res / "drawable/ic_pc_remote_notification.xml"] = vector(monochrome=True, notification=True)
        outputs[res / "drawable/ic_launcher_background.xml"] = (
            f'<vector xmlns:android="{ANDROID_NS}" android:width="108dp" android:height="108dp" '
            'android:viewportWidth="108" android:viewportHeight="108">\n'
            f'  <path android:fillColor="{BG}" android:pathData="M0 0H108V108H0Z"/>\n</vector>\n').encode()
        for api in (26, 33):
            monochrome = ('  <monochrome android:drawable="@drawable/ic_launcher_monochrome"/>\n' if api >= 33 else "")
            adaptive = (f'<adaptive-icon xmlns:android="{ANDROID_NS}">\n'
                        '  <background android:drawable="@drawable/ic_launcher_background"/>\n'
                        '  <foreground android:drawable="@drawable/ic_launcher_foreground"/>\n'
                        f'{monochrome}</adaptive-icon>\n').encode()
            for name in ("ic_launcher.xml", "ic_launcher_round.xml"):
                outputs[res / f"mipmap-anydpi-v{api}" / name] = adaptive
        # A legacy launcher displays the whole bitmap; match adaptive visible scale.
        for density, size in (("mdpi", 48), ("hdpi", 72), ("xhdpi", 96), ("xxhdpi", 144), ("xxxhdpi", 192)):
            for name, rounded in (("ic_launcher.png", False), ("ic_launcher_round.png", True)):
                outputs[res / f"mipmap-{density}" / name] = render(svg(background=BG, zoom=True, round_icon=rounded), size)
    mismatches = []
    for path, data in outputs.items():
        if args.check:
            if not path.is_file() or path.read_bytes() != data:
                mismatches.append(str(path))
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    if mismatches:
        raise SystemExit("Stale assets:\n" + "\n".join(mismatches))
    print(f"{'Verified' if args.check else 'Generated'} {len(outputs)} assets; ICO PNG layers: {ICO_SIZES}")


if __name__ == "__main__":
    main()
