# PC Remote / Precision Nexus

A floating display horizon and fluid handheld remote share a minimalist, clean geometric silhouette.
Ice (`#F8FAFC`) identifies the computer; cyan (`#00E5FF`) identifies the remote
controller and wireless pulse. Obsidian (`#0F141C`) provides the neutral surface. No gradients,
shadows, fonts, or external artwork are used in the mark.

`symbol.svg` is the canonical, unmasked 108 × 108 vector. Filled paths, rather
than thin strokes, preserve the silhouette at small sizes. `icon.svg` is its
desktop composition. `preview.png` shows actual desktop sizes and simulated
Android circle, squircle, rounded-square and themed masks. Inspect the native
16px and 24px layers at actual size; small details necessarily resolve to pixels.

The Android foreground stays inside the central 66dp square **and** 66dp diameter
circle: approximately x=28–80dp, y=32–78dp; maximum meaningful radius 32.67dp.
The background fills the entire 108dp layer without its own mask. Foreground,
monochrome and notification drawables all use the canonical path geometry.
API 26 resources provide adaptive foreground/background; API 33 adds the
monochrome layer. Legacy regular and round PNGs cover all five density buckets.
The legacy mark uses the desktop optical scale; the launcher applies its own
72dp visible viewport to the adaptive layers.

The Windows ICO has seven independent PNG-compressed RGBA layers: 16, 24, 32,
48, 64, 128 and 256px. Every layer is rendered directly from the vector.
Tauri PNGs are 32, 128, 256 and 512px, with a 1024px master. The sidebar and
favicon use the same generated desktop SVG. Android notifications use a
white-on-transparent vector, since a full-color adaptive launcher is unsuitable
as a notification small icon.

Reproduce from `pc-remote-desktop` (Python 3.14 used for validation):

```powershell
py -3.14 -m venv .icon-venv
.icon-venv/Scripts/python -m pip install -r brand/requirements.txt
.icon-venv/Scripts/python scripts/generate-icons.py --android ../pc-remote-android
.icon-venv/Scripts/python scripts/generate-icons.py --android ../pc-remote-android --check
```

On Linux, use `.icon-venv/bin/python` instead. The Android destination is explicit
to avoid modifying other sibling Android repositories. `--check` compares every
generated asset byte-for-byte, verifies the safe circle and never writes assets.
No app identifiers, versions, permissions, protocol or signing settings change.

Platform references: [Android adaptive icons](https://developer.android.com/develop/ui/compose/system/icon_design_adaptive).
