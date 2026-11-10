"""Render the Lighthouse brand assets (icon, logo, banner, animated icon).

The SVGs in assets/brand are generated here and are the source of truth. PNGs are rendered with a
headless Chromium-based browser (Edge or Chrome) and the GIF is assembled with ffmpeg.

Usage:
  python tools/brand/render_brand.py --browser "<path to msedge/chrome>" --ffmpeg "<path to ffmpeg>" \
      --out assets/brand --work <empty temp folder>
"""

import argparse
import math
import os
import shutil
import subprocess
import sys

# Palette (matches the dashboard's Harbor / Tide themes)
SKY_TOP = "#1d3a40"
SKY_BOTTOM = "#122023"
TOWER = "#eef3f3"
TOWER_SHADE = "#d3dfe0"
BAND = "#2f7f86"
ROOF = "#2f7f86"
LANTERN = "#173034"
LAMP = "#f3b864"
BEAM = "#f3c27e"
ROCK = "#28474d"
WATER = "#5fb3b2"
INK_LIGHT = "#1f2b2f"
INK_DARK = "#eef3f3"

FONT = "'Segoe UI Variable Display','SF Pro Rounded',ui-rounded,'Segoe UI',system-ui,sans-serif"


def beams(cx, cy, angle, reach, spread, opacity, gid):
    """Two light beams from (cx, cy). `angle` 0 = pointing right; seen from the side, so a beam's
    horizontal length is reach*cos(angle). Returns SVG for the beam(s) currently visible."""
    out = []
    c = math.cos(angle)
    for side in (1, -1):
        length = reach * c * side
        if length <= 4:
            continue
        x_end = cx + length * (1 if side == 1 else -1)
        half_near, half_far = 7, spread
        pts = f"{cx},{cy - half_near} {x_end:.1f},{cy - half_far} {x_end:.1f},{cy + half_far} {cx},{cy + half_near}"
        grad = f"{gid}{'r' if side == 1 else 'l'}"
        out.append(
            f'<linearGradient id="{grad}" x1="{cx}" y1="0" x2="{x_end:.1f}" y2="0" gradientUnits="userSpaceOnUse">'
            f'<stop offset="0" stop-color="{BEAM}" stop-opacity="{opacity:.2f}"/>'
            f'<stop offset="1" stop-color="{BEAM}" stop-opacity="0"/></linearGradient>'
            f'<polygon points="{pts}" fill="url(#{grad})"/>'
        )
    return "".join(out)


def lighthouse(x, y, s, beam_angle=0.0, beam_reach=230, glow=1.0, beam_opacity=0.55, gid="b"):
    """The mark, drawn with its lamp at (x, y), scaled by s (1.0 = 512 px icon size)."""
    def X(v):
        return x + v * s

    def Y(v):
        return y + v * s

    lamp_glow = 34 + 18 * glow
    parts = [
        f'<radialGradient id="{gid}g" cx="{x}" cy="{y}" r="{lamp_glow * s:.1f}" gradientUnits="userSpaceOnUse">'
        f'<stop offset="0" stop-color="{LAMP}" stop-opacity="{0.55 * glow + 0.15:.2f}"/>'
        f'<stop offset="1" stop-color="{LAMP}" stop-opacity="0"/></radialGradient>',
        beams(x, y, beam_angle, beam_reach * s, 30 * s, beam_opacity, gid),
        # tower (tapered), drawn from the gallery down to the rock
        f'<path d="M{X(-36)},{Y(30)} L{X(36)},{Y(30)} L{X(62)},{Y(226)} Q{X(62)},{Y(232)} {X(56)},{Y(232)} '
        f'L{X(-56)},{Y(232)} Q{X(-62)},{Y(232)} {X(-62)},{Y(226)} Z" fill="{TOWER}"/>',
        # right-hand shade for a little depth
        f'<path d="M{X(10)},{Y(30)} L{X(36)},{Y(30)} L{X(62)},{Y(226)} Q{X(62)},{Y(232)} {X(56)},{Y(232)} '
        f'L{X(22)},{Y(232)} Z" fill="{TOWER_SHADE}" opacity=".55"/>',
        # bands
        f'<path d="M{X(-43.5)},{Y(86)} L{X(43.5)},{Y(86)} L{X(47.4)},{Y(116)} L{X(-47.4)},{Y(116)} Z" fill="{BAND}"/>',
        f'<path d="M{X(-51.3)},{Y(146)} L{X(51.3)},{Y(146)} L{X(55.2)},{Y(176)} L{X(-55.2)},{Y(176)} Z" fill="{BAND}"/>',
        # gallery deck
        f'<rect x="{X(-52)}" y="{Y(20)}" width="{104 * s:.1f}" height="{12 * s:.1f}" rx="{4 * s:.1f}" fill="{TOWER}"/>',
        # lantern room
        f'<rect x="{X(-30)}" y="{Y(-26)}" width="{60 * s:.1f}" height="{48 * s:.1f}" rx="{9 * s:.1f}" '
        f'fill="{LANTERN}" stroke="{TOWER}" stroke-width="{6 * s:.1f}"/>',
        # roof and finial
        f'<path d="M{X(-40)},{Y(-24)} Q{X(0)},{Y(-74)} {X(40)},{Y(-24)} Z" fill="{ROOF}"/>',
        f'<circle cx="{x}" cy="{Y(-64)}" r="{6 * s:.1f}" fill="{ROOF}"/>',
        # lamp
        f'<circle cx="{x}" cy="{y}" r="{lamp_glow * s:.1f}" fill="url(#{gid}g)"/>',
        f'<circle cx="{x}" cy="{y}" r="{(11 + 2 * glow) * s:.1f}" fill="{LAMP}"/>',
        # rock
        f'<rect x="{X(-88)}" y="{Y(228)}" width="{176 * s:.1f}" height="{22 * s:.1f}" rx="{11 * s:.1f}" fill="{ROCK}"/>',
    ]
    return "".join(parts)


def waves(x0, x1, y, s, opacity, amp=7, period=56):
    d = [f"M{x0},{y}"]
    steps = int((x1 - x0) / (period / 2 * s)) + 1
    for i in range(steps):
        xa = x0 + (i + 0.5) * period / 2 * s
        xb = x0 + (i + 1) * period / 2 * s
        dy = -amp * s if i % 2 == 0 else amp * s
        d.append(f"Q{xa:.1f},{y + dy:.1f} {min(xb, x1):.1f},{y}")
    return (f'<path d="{" ".join(d)}" fill="none" stroke="{WATER}" stroke-width="{7 * s:.1f}" '
            f'stroke-linecap="round" opacity="{opacity}"/>')


def icon_svg(beam_angle=0.0, glow=1.0, size=512):
    s = size / 512
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}">'
        f'<defs><linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">'
        f'<stop offset="0" stop-color="{SKY_TOP}"/><stop offset="1" stop-color="{SKY_BOTTOM}"/></linearGradient>'
        f'<clipPath id="card"><rect width="{size}" height="{size}" rx="{112 * s:.1f}"/></clipPath></defs>'
        f'<rect width="{size}" height="{size}" rx="{112 * s:.1f}" fill="url(#sky)"/>'
        f'<g clip-path="url(#card)">'
        + lighthouse(256 * s, 168 * s, 0.92 * s, beam_angle=beam_angle, glow=glow, gid="i")
        + waves(96 * s, 416 * s, 430 * s, s, 0.55)
        + waves(150 * s, 362 * s, 462 * s, s, 0.3)
        + "</g></svg>"
    )


def logo_svg(dark_text=True):
    ink = INK_LIGHT if dark_text else INK_DARK
    w, h = 1200, 360
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">'
        f'<g transform="translate(24,4) scale(0.68)">' + icon_svg()[icon_svg().index(">") + 1:-6] + "</g>"
        f'<text x="400" y="222" font-family="{FONT}" font-size="150" font-weight="650" letter-spacing="-3" '
        f'fill="{ink}">Lighthouse</text>'
        "</svg>"
    )


def banner_svg():
    w, h = 1280, 400
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">'
        f'<defs><linearGradient id="bsky" x1="0" y1="0" x2="0" y2="1">'
        f'<stop offset="0" stop-color="{SKY_TOP}"/><stop offset="1" stop-color="{SKY_BOTTOM}"/></linearGradient></defs>'
        f'<rect width="{w}" height="{h}" fill="url(#bsky)"/>'
        + "".join(f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{INK_DARK}" opacity="{o}"/>'
                  for cx, cy, r, o in [(470, 70, 2, .5), (620, 46, 1.6, .35), (910, 84, 2, .45), (1090, 52, 1.5, .3),
                                       (1210, 120, 2, .4), (760, 118, 1.4, .3), (980, 160, 1.4, .25)])
        + lighthouse(250, 150, 0.78, beam_angle=0.0, beam_reach=1250, beam_opacity=0.32, gid="bn")
        + waves(-20, 1300, 350, 1, 0.5, amp=6, period=64)
        + waves(-20, 1300, 380, 1, 0.25, amp=6, period=64)
        + f'<text x="440" y="208" font-family="{FONT}" font-size="104" font-weight="650" letter-spacing="-2" '
          f'fill="{INK_DARK}">Lighthouse</text>'
        + f'<text x="444" y="262" font-family="{FONT}" font-size="32" font-weight="400" fill="{INK_DARK}" '
          f'opacity=".72">Calm, clear game server status for Discord.</text>'
        + "</svg>"
    )


def mark_svg(size=64):
    """The lighthouse alone (no background card), for headers and favicons."""
    s = size / 300
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}">'
        + lighthouse(150 * s, 62 * s, 0.8 * s, beam_reach=170, beam_opacity=0.6, gid="m")
        + "</svg>"
    )


def hero_svg():
    """Wide illustration for the dashboard hero: lighthouse on the right, beam reaching left."""
    w, h = 900, 300
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" '
        f'preserveAspectRatio="xMaxYMax meet">'
        + "".join(f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{INK_DARK}" opacity="{o}"/>'
                  for cx, cy, r, o in [(180, 50, 1.6, .45), (330, 30, 1.2, .3), (520, 70, 1.6, .4), (640, 26, 1.2, .3)])
        + lighthouse(760, 92, 0.62, beam_reach=0, gid="h")  # the dashboard animates its own beam
        + waves(300, 920, 262, 1, 0.45, amp=5, period=58)
        + waves(420, 920, 286, 1, 0.22, amp=5, period=58)
        + "</svg>"
    )


def wave_mask_svg():
    """One seamless wave period (repeats horizontally) used as a CSS mask for the sea."""
    return ('<svg xmlns="http://www.w3.org/2000/svg" width="240" height="40" viewBox="0 0 240 40">'
            '<path d="M0 20 Q30 6 60 20 T120 20 T180 20 T240 20 V40 H0 Z" fill="#fff"/></svg>')


def render(browser, svg_path, png_path, width, height, transparent, work):
    png_path = os.path.abspath(png_path)
    args = [browser, "--headless=new", "--disable-gpu", "--hide-scrollbars",
            f"--user-data-dir={os.path.join(work, 'browser-profile')}",
            f"--window-size={width},{height}", f"--screenshot={png_path}"]
    if transparent:
        args.append("--default-background-color=00000000")
    args.append("file:///" + os.path.abspath(svg_path).replace("\\", "/"))
    flags = 0x00004000 if sys.platform == "win32" else 0  # BELOW_NORMAL_PRIORITY_CLASS
    subprocess.run(args, check=True, creationflags=flags, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                   timeout=60)
    if not os.path.exists(png_path):
        raise RuntimeError(f"browser did not write {png_path}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--browser", required=True)
    ap.add_argument("--ffmpeg", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--frames", type=int, default=48)
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    os.makedirs(a.work, exist_ok=True)

    def write(name, svg):
        p = os.path.join(a.out, name)
        with open(p, "w", encoding="utf-8", newline="\n") as f:
            f.write(svg)
        return p

    icon = write("lighthouse-icon.svg", icon_svg())
    logo = write("lighthouse-logo.svg", logo_svg(dark_text=True))
    logo_dark = write("lighthouse-logo-on-dark.svg", logo_svg(dark_text=False))
    banner = write("lighthouse-banner.svg", banner_svg())
    write("lighthouse-mark.svg", mark_svg())
    write("lighthouse-hero.svg", hero_svg())
    write("wave.svg", wave_mask_svg())

    render(a.browser, icon, os.path.join(a.out, "lighthouse-icon-512.png"), 512, 512, True, a.work)
    for size in (256, 128, 64):
        p = write_tmp = os.path.join(a.work, f"icon-{size}.svg")
        with open(p, "w", encoding="utf-8") as f:
            f.write(icon_svg(size=size))
        render(a.browser, write_tmp, os.path.join(a.out, f"lighthouse-icon-{size}.png"), size, size, True, a.work)
    render(a.browser, logo, os.path.join(a.out, "lighthouse-logo.png"), 1200, 360, True, a.work)
    render(a.browser, logo_dark, os.path.join(a.out, "lighthouse-logo-on-dark.png"), 1200, 360, True, a.work)
    render(a.browser, banner, os.path.join(a.out, "lighthouse-banner.png"), 1280, 400, False, a.work)

    # Animated icon: the beam turns once per loop; the lamp brightens as it faces the viewer.
    frames = os.path.join(a.work, "frames")
    os.makedirs(frames, exist_ok=True)
    for i in range(a.frames):
        t = 2 * math.pi * i / a.frames
        glow = 0.35 + 0.65 * max(0.0, math.sin(t)) ** 2
        svg = os.path.join(frames, f"f{i:03d}.svg")
        with open(svg, "w", encoding="utf-8") as f:
            f.write(icon_svg(beam_angle=t, glow=glow, size=256))
        render(a.browser, svg, os.path.join(frames, f"f{i:03d}.png"), 256, 256, True, a.work)
    gif = os.path.join(a.out, "lighthouse-animated.gif")
    pattern = os.path.join(frames, "f%03d.png")
    palette = os.path.join(a.work, "palette.png")
    fps = 16
    subprocess.run([a.ffmpeg, "-y", "-loglevel", "error", "-framerate", str(fps), "-i", pattern,
                    "-vf", "palettegen=max_colors=128:stats_mode=full:reserve_transparent=1", palette], check=True)
    subprocess.run([a.ffmpeg, "-y", "-loglevel", "error", "-framerate", str(fps), "-i", pattern, "-i", palette,
                    "-lavfi", "paletteuse=dither=sierra2_4a:alpha_threshold=128", "-loop", "0",
                    "-gifflags", "-offsetting", gif], check=True)
    shutil.rmtree(frames, ignore_errors=True)
    print("wrote", sorted(os.listdir(a.out)))


if __name__ == "__main__":
    main()
