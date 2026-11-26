#!/usr/bin/env python3
"""Turn the raw window recording + driver log into the finished light-mode demo video.

usage: compose.py <raw.mov> <driver.log> <work-dir> <assets-bin> <icon> <out.mp4>
"""

import json
import os
import subprocess
import sys

RAW, LOG, WORK, ASSETS, ICON, OUT = sys.argv[1:7]
FPS = 60
CANVAS_W, CANVAS_H = 3200, 1800
INTRO, OUTRO, XFADE = 2.8, 3.2, 0.55
CAP_FADE = 0.3


def probe_start(path):
    with open(path + ".start") as f:
        return float(f.read().strip())


def probe_video_size(path):
    raw = subprocess.check_output(
        [
            "ffprobe",
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0:s=x",
            path,
        ],
        text=True,
    ).strip()
    w, h = raw.split("x")
    return int(w), int(h)


def load_events(path):
    with open(path) as f:
        return [json.loads(line) for line in f if line.strip()]


def smootherstep(t0, dur):
    u = f"clip((it-{t0:.4f})/{max(dur, 0.01):.4f},0,1)"
    return f"(st(1,{u})*0+ld(1)*ld(1)*ld(1)*(ld(1)*(ld(1)*6-15)+10))"


def piecewise(base, steps):
    expr = f"{base:.4f}"
    for t0, dur, delta in steps:
        if abs(delta) > 1e-6:
            expr += f"+({delta:.4f})*{smootherstep(t0, dur)}"
    return expr


def main():
    r0 = probe_start(RAW)
    win_w, win_h = probe_video_size(RAW)
    scale = 2 if win_w >= 2000 else 1
    win_ox = max(0, (CANVAS_W - win_w) // 2)
    win_oy = 110 if CANVAS_H - win_h >= 240 else max(0, (CANVAS_H - win_h) // 2)

    events = load_events(LOG)
    marks = [e for e in events if e["ev"] == "mark"]
    begin = next(e["t"] for e in marks if e["id"] == "begin")
    end = next(e["t"] for e in marks if e["id"] == "end")
    length = end - begin
    rel = lambda t: t - begin  # noqa: E731

    z_prev, cx_prev, cy_prev = 1.0, CANVAS_W / 2, CANVAS_H / 2
    zs, xs, ys = [], [], []
    for e in events:
        if e["ev"] != "cam":
            continue
        z = float(e["zoom"])
        cx = win_ox + scale * float(e["x"]) if z > 1.001 else CANVAS_W / 2
        cy = win_oy + scale * float(e["y"]) if z > 1.001 else CANVAS_H / 2
        t0, dur = rel(e["t"]), float(e.get("dur", 0.9))
        zs.append((t0, dur, z - z_prev))
        xs.append((t0, dur, cx - cx_prev))
        ys.append((t0, dur, cy - cy_prev))
        z_prev, cx_prev, cy_prev = z, cx, cy
    z_expr = piecewise(1.0, zs)
    x_expr = f"clip(({piecewise(CANVAS_W / 2, xs)})-iw/zoom/2,0,iw-iw/zoom)"
    y_expr = f"clip(({piecewise(CANVAS_H / 2, ys)})-ih/zoom/2,0,ih-ih/zoom)"

    captions = []
    for i, m in enumerate(marks):
        if not m.get("en"):
            continue
        nxt = marks[i + 1]["t"] if i + 1 < len(marks) else end
        start, stop = rel(m["t"]) + 0.12, rel(nxt) - 0.1
        if stop - start < 1.0:
            continue
        idx = f"{len(captions) + 1:02d}"
        png = os.path.join(WORK, f"caption_{idx}.png")
        subprocess.check_call([ASSETS, "caption", png, m["en"], m.get("zh", ""), idx])
        captions.append((png, start, stop - start))

    frame_png = os.path.join(WORK, "frame.png")
    intro_png = os.path.join(WORK, "intro.png")
    outro_png = os.path.join(WORK, "outro.png")
    subprocess.check_call([ASSETS, "frame", frame_png, str(win_w), str(win_h), str(win_ox), str(win_oy)])
    subprocess.check_call([ASSETS, "card", "intro", intro_png, ICON])
    subprocess.check_call([ASSETS, "card", "outro", outro_png, ICON])

    inputs = [
        "-ss", f"{begin - r0:.4f}", "-t", f"{length:.4f}", "-i", RAW,
        "-loop", "1", "-framerate", str(FPS), "-i", frame_png,
        "-loop", "1", "-framerate", str(FPS), "-t", str(INTRO), "-i", intro_png,
        "-loop", "1", "-framerate", str(FPS), "-t", str(OUTRO), "-i", outro_png,
    ]
    for png, _, dur in captions:
        inputs += ["-loop", "1", "-framerate", str(FPS), "-t", f"{dur:.3f}", "-i", png]

    g = []
    g.append(f"color=c=0xf1f5f9:s={CANVAS_W}x{CANVAS_H}:r={FPS}[base]")
    g.append(f"[0:v]setpts=PTS-STARTPTS,fps={FPS},format=rgba[win]")
    g.append(f"[base][win]overlay={win_ox}:{win_oy}:shortest=1[w1]")
    g.append("[w1][1:v]overlay=0:0:shortest=1[w2]")
    g.append(
        f"[w2]zoompan=z='{z_expr}':x='{x_expr}':y='{y_expr}':d=1:s=1920x1080:fps={FPS},"
        "setsar=1,format=rgba[cam0]"
    )
    cur = "cam0"
    for i, (_, start, dur) in enumerate(captions):
        k = 4 + i
        g.append(
            f"[{k}:v]format=rgba,fade=t=in:st=0:d={CAP_FADE}:alpha=1,"
            f"fade=t=out:st={dur - CAP_FADE:.3f}:d={CAP_FADE}:alpha=1,"
            f"setpts=PTS-STARTPTS+{start:.3f}/TB[c{i}]"
        )
        g.append(f"[{cur}][c{i}]overlay=0:0:eof_action=pass[cam{i + 1}]")
        cur = f"cam{i + 1}"
    norm = f"fps={FPS},format=yuv420p,setsar=1,settb=AVTB"
    g.append(f"[{cur}]{norm}[main]")
    g.append(f"[2:v]{norm}[intro]")
    g.append(f"[3:v]{norm}[outro]")
    g.append(f"[intro][main]xfade=transition=fade:duration={XFADE}:offset={INTRO - XFADE}[im]")
    g.append(
        f"[im][outro]xfade=transition=fade:duration={XFADE}:"
        f"offset={INTRO - XFADE + length - XFADE:.3f},format=yuv420p[v]"
    )

    script = os.path.join(WORK, "graph.txt")
    with open(script, "w") as f:
        f.write(";\n".join(g))

    cmd = [
        "ffmpeg", "-y", "-v", "error", "-stats", *inputs, "-/filter_complex", script,
        "-map", "[v]", "-c:v", "libx264", "-preset", "slow", "-crf", "18",
        "-pix_fmt", "yuv420p", "-movflags", "+faststart", OUT,
    ]
    print(f"==> win {win_w}x{win_h} @ ({win_ox},{win_oy}), main {length:.1f}s, {len(captions)} captions, {len(zs)} camera moves")
    subprocess.check_call(cmd)


if __name__ == "__main__":
    main()
