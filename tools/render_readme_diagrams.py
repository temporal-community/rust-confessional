#!/usr/bin/env python3
"""Render the README architecture SVG and the redeploy comparison GIF."""

from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont


ROOT = Path(__file__).resolve().parents[1]
MEDIA = ROOT / "docs" / "media"

BG = "#0b0d14"
PANEL = "#151722"
NODE = "#222536"
NODE_STROKE = "#44485f"
TEXT = "#f4f3f1"
MUTED = "#9da1b5"
FAINT = "#5f6378"
ORANGE = "#ff6b3d"
ORANGE_2 = "#ff9b54"
PURPLE = "#8b7cf6"
BLUE = "#58b8ff"
GREEN = "#42d39c"
RED = "#ff5f6d"


def font_path(bold: bool = False, mono: bool = False) -> str:
    candidates = (
        [
            "/System/Library/Fonts/Supplemental/Andale Mono.ttf",
            "/System/Library/Fonts/Menlo.ttc",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        ]
        if mono
        else [
            (
                "/System/Library/Fonts/Supplemental/Verdana Bold.ttf"
                if bold
                else "/System/Library/Fonts/Supplemental/Verdana.ttf"
            ),
            (
                "/System/Library/Fonts/Supplemental/Arial Bold.ttf"
                if bold
                else "/System/Library/Fonts/Supplemental/Arial.ttf"
            ),
            (
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"
                if bold
                else "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
            ),
        ]
    )
    for candidate in candidates:
        if Path(candidate).exists():
            return candidate
    raise FileNotFoundError("A supported system font was not found")


def get_font(size: int, bold: bool = False, mono: bool = False) -> ImageFont.FreeTypeFont:
    return ImageFont.truetype(font_path(bold=bold, mono=mono), size)


FONTS = {
    "title": get_font(31, bold=True),
    "subtitle": get_font(15),
    "panel": get_font(13, bold=True),
    "node": get_font(17, bold=True),
    "small": get_font(13),
    "tiny": get_font(11),
    "mono": get_font(12, mono=True),
    "phase": get_font(15, bold=True),
}


def hex_rgb(value: str) -> tuple[int, int, int]:
    value = value.lstrip("#")
    return tuple(int(value[i : i + 2], 16) for i in (0, 2, 4))


def rgba(value: str, alpha: int = 255) -> tuple[int, int, int, int]:
    return (*hex_rgb(value), alpha)


def ease(value: float) -> float:
    value = max(0.0, min(1.0, value))
    return value * value * (3.0 - 2.0 * value)


def phase_progress(t: float, start: float, end: float) -> float:
    if t <= start:
        return 0.0
    if t >= end:
        return 1.0
    return ease((t - start) / (end - start))


def bezier(
    p0: tuple[float, float],
    p1: tuple[float, float],
    p2: tuple[float, float],
    p3: tuple[float, float],
    steps: int = 48,
) -> list[tuple[float, float]]:
    points = []
    for index in range(steps + 1):
        u = index / steps
        inv = 1.0 - u
        x = (
            inv**3 * p0[0]
            + 3 * inv**2 * u * p1[0]
            + 3 * inv * u**2 * p2[0]
            + u**3 * p3[0]
        )
        y = (
            inv**3 * p0[1]
            + 3 * inv**2 * u * p1[1]
            + 3 * inv * u**2 * p2[1]
            + u**3 * p3[1]
        )
        points.append((x, y))
    return points


def point_on(points: list[tuple[float, float]], progress: float) -> tuple[float, float]:
    progress = max(0.0, min(1.0, progress))
    index = progress * (len(points) - 1)
    low = int(index)
    high = min(low + 1, len(points) - 1)
    weight = index - low
    return (
        points[low][0] * (1 - weight) + points[high][0] * weight,
        points[low][1] * (1 - weight) + points[high][1] * weight,
    )


def text(
    draw: ImageDraw.ImageDraw,
    xy: tuple[float, float],
    value: str,
    font: ImageFont.FreeTypeFont,
    fill: str | tuple[int, int, int, int],
    anchor: str = "la",
) -> None:
    draw.text(xy, value, font=font, fill=fill, anchor=anchor)


def glow_box(
    image: Image.Image,
    rect: tuple[int, int, int, int],
    *,
    fill: str,
    stroke: str,
    radius: int = 14,
    glow: float = 0.0,
    alpha: int = 255,
) -> None:
    if glow > 0:
        layer = Image.new("RGBA", image.size, (0, 0, 0, 0))
        layer_draw = ImageDraw.Draw(layer)
        layer_draw.rounded_rectangle(
            rect,
            radius=radius,
            outline=rgba(stroke, int(180 * glow)),
            width=7,
        )
        layer = layer.filter(ImageFilter.GaussianBlur(10))
        image.alpha_composite(layer)
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle(
        rect,
        radius=radius,
        fill=rgba(fill, alpha),
        outline=rgba(stroke, alpha),
        width=2,
    )


def panel(
    image: Image.Image,
    rect: tuple[int, int, int, int],
    eyebrow: str,
    accent: str,
) -> None:
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle(rect, radius=22, fill=rgba(PANEL), outline=rgba("#2a2d3d"), width=2)
    x1, y1, _, _ = rect
    draw.rounded_rectangle((x1 + 24, y1 + 24, x1 + 34, y1 + 34), radius=5, fill=rgba(accent))
    text(draw, (x1 + 44, y1 + 29), eyebrow, FONTS["panel"], accent, "lm")


def node(
    image: Image.Image,
    rect: tuple[int, int, int, int],
    title_value: str,
    subtitle_value: str,
    *,
    accent: str = NODE_STROKE,
    glow: float = 0.0,
    alpha: int = 255,
) -> None:
    layer = Image.new("RGBA", image.size, (0, 0, 0, 0))
    glow_box(
        layer,
        rect,
        fill=NODE,
        stroke=accent,
        radius=14,
        glow=glow,
        alpha=255,
    )
    draw = ImageDraw.Draw(layer)
    x1, y1, x2, y2 = rect
    text(draw, ((x1 + x2) / 2, y1 + 34), title_value, FONTS["node"], TEXT, "mm")
    text(
        draw,
        ((x1 + x2) / 2, y2 - 29),
        subtitle_value,
        FONTS["small"],
        MUTED,
        "mm",
    )
    if alpha < 255:
        layer_alpha = layer.getchannel("A").point(lambda value: value * alpha // 255)
        layer.putalpha(layer_alpha)
    image.alpha_composite(layer)


def path(
    image: Image.Image,
    points: list[tuple[float, float]],
    *,
    color: str = FAINT,
    width: int = 3,
    alpha: int = 175,
    glow: float = 0.0,
) -> None:
    if glow > 0:
        layer = Image.new("RGBA", image.size, (0, 0, 0, 0))
        ld = ImageDraw.Draw(layer)
        ld.line(points, fill=rgba(color, int(190 * glow)), width=width + 8, joint="curve")
        layer = layer.filter(ImageFilter.GaussianBlur(9))
        image.alpha_composite(layer)
    draw = ImageDraw.Draw(image)
    draw.line(points, fill=rgba(color, alpha), width=width, joint="curve")

    # A small directional marker keeps the flow readable when the GIF is
    # paused or shown as a static preview.
    end_x, end_y = points[-1]
    prev_x, prev_y = points[-3]
    delta_x, delta_y = end_x - prev_x, end_y - prev_y
    distance = max((delta_x**2 + delta_y**2) ** 0.5, 0.001)
    unit_x, unit_y = delta_x / distance, delta_y / distance
    base_x, base_y = end_x - 10 * unit_x, end_y - 10 * unit_y
    perp_x, perp_y = -unit_y, unit_x
    draw.polygon(
        (
            (end_x, end_y),
            (base_x + 5 * perp_x, base_y + 5 * perp_y),
            (base_x - 5 * perp_x, base_y - 5 * perp_y),
        ),
        fill=rgba(color, alpha),
    )


def particle(
    image: Image.Image,
    points: list[tuple[float, float]],
    progress: float,
    *,
    color: str = ORANGE_2,
) -> None:
    x, y = point_on(points, progress)
    layer = Image.new("RGBA", image.size, (0, 0, 0, 0))
    ld = ImageDraw.Draw(layer)
    for radius, alpha in ((22, 35), (14, 70), (8, 150)):
        ld.ellipse((x - radius, y - radius, x + radius, y + radius), fill=rgba(color, alpha))
    layer = layer.filter(ImageFilter.GaussianBlur(5))
    image.alpha_composite(layer)
    ImageDraw.Draw(image).ellipse((x - 4, y - 4, x + 4, y + 4), fill=rgba("#fff5dc"))


def chip(
    image: Image.Image,
    xy: tuple[int, int],
    label: str,
    color: str,
    alpha: int = 255,
) -> None:
    layer = Image.new("RGBA", image.size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    x, y = xy
    bbox = draw.textbbox((0, 0), label, font=FONTS["mono"])
    width = bbox[2] - bbox[0] + 24
    draw.rounded_rectangle(
        (x, y, x + width, y + 28),
        radius=8,
        fill=rgba(color, 36),
        outline=rgba(color, 190),
        width=1,
    )
    text(draw, (x + 12, y + 14), label, FONTS["mono"], color, "lm")
    if alpha < 255:
        layer_alpha = layer.getchannel("A").point(lambda value: value * alpha // 255)
        layer.putalpha(layer_alpha)
    image.alpha_composite(layer)


def draw_header(image: Image.Image, active_phase: int) -> None:
    draw = ImageDraw.Draw(image)
    text(
        draw,
        (48, 18),
        "RUST CONFESSIONAL  /  ONE CONFESSION, TWO OUTCOMES",
        FONTS["panel"],
        ORANGE,
    )
    text(
        draw,
        (48, 50),
        "Can Ferris finish after a Rust Worker redeploy?",
        FONTS["title"],
        TEXT,
    )
    text(
        draw,
        (48, 88),
        'Confession: "I fixed the race condition with a sleep."',
        FONTS["subtitle"],
        MUTED,
    )
    for index in range(4):
        x = 1015 + index * 34
        color = ORANGE if index <= active_phase else "#36394a"
        draw.ellipse((x, 52, x + 12, 64), fill=rgba(color))


def draw_animation_frame(t: float) -> Image.Image:
    image = Image.new("RGBA", (1200, 675), rgba(BG))
    redeploy = phase_progress(t, 1.4, 2.7)
    signal = phase_progress(t, 2.8, 4.6)
    restart = phase_progress(t, 4.7, 6.2)
    resume = phase_progress(t, 6.3, 8.2)
    active_phase = 0 if t < 1.4 else 1 if t < 2.8 else 2 if t < 6.3 else 3
    draw_header(image, active_phase)

    left = (35, 112, 585, 598)
    right = (615, 112, 1165, 598)
    panel(image, left, "PROCESS-ONLY RUST AGENT", RED)
    panel(image, right, "DURABLE RUST AGENT", PURPLE)

    draw = ImageDraw.Draw(image)
    text(
        draw,
        (62, 162),
        "Ferris's draft exists only in RAM",
        FONTS["subtitle"],
        MUTED,
    )
    text(
        draw,
        (642, 162),
        "Ferris's state is recorded by Temporal",
        FONTS["subtitle"],
        MUTED,
    )

    left_signal = (72, 238, 225, 318)
    right_signal = (652, 238, 805, 318)
    left_worker = (342, 214, 535, 340)
    right_worker = (922, 214, 1115, 340)
    temporal = (860, 392, 1115, 518)

    node(image, left_signal, "Release", "human input", accent=BLUE)
    node(image, right_signal, "Release", "human input", accent=BLUE)

    old_alpha = int(255 * (1 - redeploy))
    if old_alpha > 4:
        node(
            image,
            left_worker,
            "Ferris",
            "draft held in RAM",
            accent=ORANGE,
            glow=0.8 * (1 - redeploy),
            alpha=old_alpha,
        )
        node(
            image,
            right_worker,
            "Ferris",
            "Rust agent loop",
            accent=ORANGE,
            glow=0.8 * (1 - redeploy),
            alpha=old_alpha,
        )

    left_route = bezier((225, 278), (270, 278), (305, 278), (342, 278))
    right_signal_route = bezier((805, 278), (850, 278), (860, 350), (930, 392))
    temporal_worker_route = bezier((1030, 392), (1060, 370), (1030, 350), (1018, 340))

    path(image, left_route, color=FAINT, alpha=150)
    path(image, right_signal_route, color=FAINT, alpha=150)
    path(image, temporal_worker_route, color=PURPLE, alpha=170)

    node(
        image,
        temporal,
        "Temporal",
        "Ferris state + Signal",
        accent=PURPLE,
        glow=0.35 + 0.5 * signal,
    )
    chip(image, (886, 524), "WORKFLOW HISTORY", PURPLE)

    if t < 1.4:
        chip(image, (359, 356), "REPLY: PARKED", ORANGE)
        chip(image, (939, 356), "REPLY: PARKED", ORANGE)

    if 1.4 <= t < 2.9:
        fade = int(255 * min(1.0, redeploy + 0.15))
        chip(image, (182, 375), "RUST WORKER REDEPLOY", ORANGE, fade)
        chip(image, (788, 554), "HISTORY REMAINS", PURPLE, fade)

    if signal > 0:
        particle(image, left_route, signal, color=ORANGE_2)
        particle(image, right_signal_route, signal, color=ORANGE_2)
        path(image, left_route, color=RED, alpha=int(220 * signal), glow=signal)
        path(
            image,
            right_signal_route,
            color=ORANGE,
            alpha=int(225 * signal),
            glow=signal,
        )

    if signal >= 0.93 and restart < 0.55:
        chip(image, (72, 350), "SIGNAL LOST", RED)
        chip(image, (892, 558), "SIGNAL RECORDED", ORANGE)

    if restart > 0:
        new_alpha = int(255 * restart)
        node(
            image,
            left_worker,
            "Fresh Rust Worker",
            "Ferris state is gone",
            accent=RED,
            glow=0.25 * restart,
            alpha=new_alpha,
        )
        node(
            image,
            right_worker,
            "Fresh Rust Worker",
            "replays Ferris state",
            accent=GREEN if resume > 0.5 else ORANGE,
            glow=(0.25 + 0.65 * resume) * restart,
            alpha=new_alpha,
        )
        chip(image, (365, 361), "STATE: EMPTY", RED, new_alpha)
        chip(image, (943, 356), "STATE: RESTORED", GREEN, new_alpha)

    if resume > 0:
        path(
            image,
            temporal_worker_route,
            color=GREEN,
            alpha=int(235 * resume),
            glow=resume,
        )
        particle(image, temporal_worker_route, resume, color=GREEN)

    if resume > 0.72:
        done = phase_progress(resume, 0.72, 1.0)
        chip(image, (382, 438), "NO JUDGMENT", RED, int(255 * done))
        chip(
            image,
            (650, 558),
            "RUST FIX: USE TYPED CHANNELS",
            GREEN,
            int(255 * done),
        )
        chip(image, (982, 558), "SENT", GREEN, int(255 * done))

    phase_labels = (
        ("1  Ferris parks the reply", t < 1.4),
        ("2  Rust Worker redeploy", 1.4 <= t < 2.8),
        ("3  Release Signal during gap", 2.8 <= t < 6.3),
        ("4  Ferris resumes and sends the Rust fix", t >= 6.3),
    )
    active = next((label for label, selected in phase_labels if selected), phase_labels[-1][0])
    draw.rounded_rectangle((35, 620, 1165, 658), radius=12, fill=rgba("#11131c"))
    text(draw, (600, 639), active, FONTS["phase"], ORANGE, "mm")
    return image.convert("RGB")


def render_animation() -> None:
    fps = 10
    duration = 9.4
    frames = [draw_animation_frame(index / fps) for index in range(int(duration * fps))]
    # Hold the final outcome long enough to read before the loop restarts.
    frames.extend([frames[-1].copy() for _ in range(12)])
    gif_path = MEDIA / "durable-agent-redeploy.gif"
    frames[0].save(
        gif_path,
        save_all=True,
        append_images=frames[1:],
        duration=int(1000 / fps),
        loop=0,
        optimize=True,
        disposal=2,
    )


def architecture_svg() -> str:
    return """<svg xmlns="http://www.w3.org/2000/svg" width="1400" height="780" viewBox="0 0 1400 780" role="img" aria-labelledby="title desc">
  <title id="title">Wall of Regrets durable agent architecture</title>
  <desc id="desc">Audience input enters the Rust Stage, which starts or signals a Temporal Workflow. A Rust Worker replays deterministic Workflow state and runs model, catalog, projection, and delivery Activities. Temporal history remains the source of truth across Worker redeploys.</desc>
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#0b0d14"/>
      <stop offset="1" stop-color="#101322"/>
    </linearGradient>
    <linearGradient id="temporal" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#241c2d"/>
      <stop offset="1" stop-color="#171925"/>
    </linearGradient>
    <filter id="glow" x="-50%" y="-50%" width="200%" height="200%">
      <feGaussianBlur stdDeviation="7" result="blur"/>
      <feMerge><feMergeNode in="blur"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
    <marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
      <path d="M 0 0 L 10 5 L 0 10 z" fill="#777b91"/>
    </marker>
    <marker id="arrow-active" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
      <path d="M 0 0 L 10 5 L 0 10 z" fill="#ff6b3d"/>
    </marker>
    <style>
      .title{font:700 32px Verdana,Arial,sans-serif;fill:#f4f3f1}
      .subtitle{font:15px Verdana,Arial,sans-serif;fill:#9da1b5}
      .eyebrow{font:700 12px Verdana,Arial,sans-serif;letter-spacing:1.4px}
      .node-title{font:700 15px Verdana,Arial,sans-serif;fill:#f4f3f1}
      .node-copy{font:12px Verdana,Arial,sans-serif;fill:#9da1b5}
      .label{font:11px "Andale Mono",monospace;fill:#b8bbcb}
      .panel{fill:#151722;stroke:#2d3042;stroke-width:2}
      .node{fill:#222536;stroke:#44485f;stroke-width:1.5}
      .edge{fill:none;stroke:#64687e;stroke-width:2;marker-end:url(#arrow)}
      .active{fill:none;stroke:#ff6b3d;stroke-width:3;marker-end:url(#arrow-active);filter:url(#glow)}
    </style>
  </defs>
  <rect width="1400" height="780" rx="24" fill="url(#bg)"/>
  <text x="42" y="53" class="title">How the durable Rust agent fits together</text>
  <text x="42" y="82" class="subtitle">Workflow code decides. Activities do. Temporal remembers between Worker tasks.</text>

  <g>
    <rect x="34" y="112" width="220" height="232" rx="20" class="panel"/>
    <circle cx="57" cy="137" r="5" fill="#58b8ff"/>
    <text x="70" y="142" class="eyebrow" fill="#58b8ff">AUDIENCE</text>
    <rect x="61" y="178" width="166" height="60" rx="12" class="node"/>
    <text x="144" y="203" text-anchor="middle" class="node-title">Phone or form</text>
    <text x="144" y="222" text-anchor="middle" class="node-copy">programming confession</text>
    <rect x="61" y="258" width="166" height="60" rx="12" class="node"/>
    <text x="144" y="283" text-anchor="middle" class="node-title">Operator</text>
    <text x="144" y="302" text-anchor="middle" class="node-copy">hold, release, reset</text>
  </g>

  <g>
    <rect x="34" y="370" width="220" height="330" rx="20" class="panel"/>
    <circle cx="57" cy="395" r="5" fill="#8b7cf6"/>
    <text x="70" y="400" class="eyebrow" fill="#8b7cf6">RUST STAGE</text>
    <rect x="61" y="436" width="166" height="68" rx="12" class="node"/>
    <text x="144" y="463" text-anchor="middle" class="node-title">Axum API</text>
    <text x="144" y="484" text-anchor="middle" class="node-copy">start + Signal</text>
    <rect x="61" y="532" width="166" height="68" rx="12" class="node"/>
    <text x="144" y="559" text-anchor="middle" class="node-title">Projection store</text>
    <text x="144" y="580" text-anchor="middle" class="node-copy">safe display state</text>
    <rect x="61" y="628" width="166" height="46" rx="12" fill="#242139" stroke="#8b7cf6" stroke-width="1.5"/>
    <text x="144" y="657" text-anchor="middle" class="node-title">Wall dashboard</text>
  </g>

  <g>
    <rect x="314" y="112" width="300" height="588" rx="20" fill="url(#temporal)" stroke="#ff6b3d" stroke-width="2"/>
    <circle cx="339" cy="137" r="5" fill="#ff6b3d"/>
    <text x="352" y="142" class="eyebrow" fill="#ff8a58">TEMPORAL</text>
    <text x="589" y="142" text-anchor="end" class="label" fill="#ff8a58">SOURCE OF TRUTH</text>
    <rect x="346" y="184" width="236" height="82" rx="14" fill="#2a202a" stroke="#ff6b3d" stroke-width="1.6"/>
    <text x="464" y="216" text-anchor="middle" class="node-title">ConfessionWorkflow</text>
    <text x="464" y="239" text-anchor="middle" class="node-copy">deterministic orchestration</text>
    <rect x="346" y="303" width="236" height="82" rx="14" class="node"/>
    <text x="464" y="335" text-anchor="middle" class="node-title">Event history</text>
    <text x="464" y="358" text-anchor="middle" class="node-copy">inputs + Activity results</text>
    <rect x="346" y="422" width="236" height="82" rx="14" class="node"/>
    <text x="464" y="454" text-anchor="middle" class="node-title">Reply Pending</text>
    <text x="464" y="477" text-anchor="middle" class="node-copy">durable human wait</text>
    <rect x="346" y="541" width="236" height="82" rx="14" fill="#2a202a" stroke="#ff6b3d" stroke-width="1.6"/>
    <text x="464" y="573" text-anchor="middle" class="node-title">release Signal</text>
    <text x="464" y="596" text-anchor="middle" class="node-copy">recorded during a gap</text>
    <path d="M464 266 V303" class="active"/>
    <path d="M464 385 V422" class="edge"/>
    <path d="M464 504 V541" class="active"/>
  </g>

  <g>
    <rect x="674" y="112" width="462" height="588" rx="20" class="panel"/>
    <circle cx="699" cy="137" r="5" fill="#ff9b54"/>
    <text x="712" y="142" class="eyebrow" fill="#ff9b54">RUST WORKER</text>
    <text x="1110" y="142" text-anchor="end" class="label">REPLAY-COMPATIBLE CODE</text>
    <rect x="720" y="196" width="154" height="68" rx="14" class="node"/>
    <text x="797" y="224" text-anchor="middle" class="node-title">Observe state</text>
    <text x="797" y="245" text-anchor="middle" class="node-copy">typed snapshot</text>
    <rect x="938" y="196" width="154" height="68" rx="14" fill="#2a202a" stroke="#ff6b3d" stroke-width="1.6"/>
    <text x="1015" y="224" text-anchor="middle" class="node-title">Decide</text>
    <text x="1015" y="245" text-anchor="middle" class="node-copy">next agent step</text>
    <rect x="938" y="354" width="154" height="68" rx="14" class="node"/>
    <text x="1015" y="382" text-anchor="middle" class="node-title">Run Activity</text>
    <text x="1015" y="403" text-anchor="middle" class="node-copy">model or skill</text>
    <rect x="720" y="354" width="154" height="68" rx="14" class="node"/>
    <text x="797" y="382" text-anchor="middle" class="node-title">Fold result</text>
    <text x="797" y="403" text-anchor="middle" class="node-copy">typed durable state</text>
    <path d="M874 230 H938" class="active"/>
    <path d="M1015 264 V354" class="active"/>
    <path d="M938 388 H874" class="active"/>
    <path d="M797 354 C715 322 715 287 797 264" class="active"/>
    <text x="906" y="306" text-anchor="middle" class="label">BOUNDED LOOP</text>
    <rect x="720" y="505" width="372" height="114" rx="16" fill="#1d2130" stroke="#42d39c" stroke-width="1.6"/>
    <text x="906" y="538" text-anchor="middle" class="node-title">Shared durable tail</text>
    <text x="906" y="566" text-anchor="middle" class="node-copy">wait for release  →  report  →  deliver</text>
    <text x="906" y="592" text-anchor="middle" class="label" fill="#42d39c">FRESH WORKER CAN RESUME HERE</text>
  </g>

  <g>
    <rect x="1172" y="112" width="194" height="588" rx="20" class="panel"/>
    <circle cx="1197" cy="137" r="5" fill="#42d39c"/>
    <text x="1210" y="142" class="eyebrow" fill="#42d39c">ACTIVITIES</text>
    <rect x="1196" y="196" width="146" height="76" rx="12" class="node"/>
    <text x="1269" y="224" text-anchor="middle" class="node-title">Model</text>
    <text x="1269" y="247" text-anchor="middle" class="node-copy">fixture / OpenAI</text>
    <rect x="1196" y="314" width="146" height="76" rx="12" class="node"/>
    <text x="1269" y="342" text-anchor="middle" class="node-title">Rust catalog</text>
    <text x="1269" y="365" text-anchor="middle" class="node-copy">approved remedies</text>
    <rect x="1196" y="432" width="146" height="76" rx="12" class="node"/>
    <text x="1269" y="460" text-anchor="middle" class="node-title">Stage report</text>
    <text x="1269" y="483" text-anchor="middle" class="node-copy">projection update</text>
    <rect x="1196" y="550" width="146" height="76" rx="12" class="node"/>
    <text x="1269" y="578" text-anchor="middle" class="node-title">Delivery</text>
    <text x="1269" y="601" text-anchor="middle" class="node-copy">deduped by ID</text>
  </g>

  <path d="M144 318 C144 362 144 397 144 436" class="edge"/>
  <path d="M254 470 C278 470 294 225 346 225" class="active"/>
  <text x="276" y="331" class="label">START</text>
  <path d="M254 480 C300 480 300 582 346 582" class="active"/>
  <text x="275" y="557" class="label">SIGNAL</text>
  <path d="M614 225 C644 225 648 230 674 230" class="active"/>
  <text x="644" y="213" class="label">WORKFLOW TASK</text>
  <path d="M614 463 C645 463 646 562 674 562" class="edge"/>
  <text x="639" y="500" text-anchor="end" class="label">WAIT / RESUME</text>
  <path d="M1092 388 C1125 388 1141 234 1196 234" class="active"/>
  <text x="1142" y="293" class="label">ACTIVITY</text>
  <path d="M1092 388 C1140 388 1140 352 1196 352" class="edge"/>
  <path d="M1092 562 C1142 562 1142 588 1196 588" class="edge"/>
  <path d="M1196 470 C1096 470 1098 672 254 652" class="edge"/>
  <text x="671" y="684" class="label">SAFE PROJECTION</text>
</svg>
"""


def render_static() -> None:
    (MEDIA / "durable-agent-architecture.svg").write_text(architecture_svg(), encoding="utf-8")


def main() -> None:
    MEDIA.mkdir(parents=True, exist_ok=True)
    render_static()
    render_animation()
    print(MEDIA / "durable-agent-architecture.svg")
    print(MEDIA / "durable-agent-redeploy.gif")


if __name__ == "__main__":
    main()
