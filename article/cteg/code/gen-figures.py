#!/usr/bin/env python3
"""Draw the data figures of the CTEG article from trace.json.

Usage: python3 gen-figures.py trace.json ../images/src
Writes 01-cover.svg, 05-raw-vs-cteg.svg, 06-real-trace.svg, 08-beyond-single-parent.svg.
The parenthood rules are the same as in cteg-demo.mjs.
"""
import json, sys
from html import escape

trace = json.load(open(sys.argv[1]))
out = sys.argv[2]

BG, INK, COCOA, MUTED, LINE = "#fffaf0", "#2a1a10", "#3b2418", "#6b5443", "#ecdcc0"
PINK, PINK_D, PINK_SOFT, GREY = "#e8607f", "#d2456a", "#ffe3ea", "#b8a58c"
FILL = {"Prompt": "#ff8fa3", "Thinking": "#fff4d8", "Message": "#fff4d8", "ToolCall": "#ffd98a",
        "ToolResult": "#f6e2bf", "SubagentCall": "#e8c9a0", "SubagentReturn": "#e8c9a0",
        "Notification": "#fff4d8", "HarnessContext": "#f3ece0"}
FONT = 'font-family="Helvetica Neue, Helvetica, Arial, sans-serif"'
MONO = 'font-family="Menlo, monospace"'
MARKERS = f'''<defs>
<marker id="ck" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M0 0L10 5L0 10z" fill="{COCOA}"/></marker>
<marker id="pk" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M0 0L10 5L0 10z" fill="{PINK_D}"/></marker>
</defs>'''

def svg(w, h, title, body):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" {FONT}>\n<title>{escape(title)}</title>\n'
            f'{MARKERS}\n<rect width="{w}" height="{h}" fill="{BG}"/>\n{body}\n</svg>\n')

def text(x, y, s, size=17, fill=INK, weight=None, anchor=None, mono=False):
    a = f' font-weight="{weight}"' if weight else ""
    a += f' text-anchor="{anchor}"' if anchor else ""
    a += f" {MONO}" if mono else ""
    return f'<text x="{x:.1f}" y="{y:.1f}" font-size="{size}" fill="{fill}"{a}>{escape(s)}</text>'

# ---- the same derivation as cteg-demo.mjs ----------------------------------
main = trace["main"]
sub = trace["subagents"][0]["events"]
allev = main + sub
by_id = {e["id"]: e for e in allev}
for e in allev:
    if e["type"] == "HarnessContext":
        e["cause"] = None
        continue
    p = by_id.get(e["parent"])
    while p and p["type"] == "HarnessContext":
        p = by_id.get(p["parent"])
    e["cause"] = p["id"] if p else None
sub_call = next(e for e in main if e["type"] == "SubagentCall")
sub_ret = next(e for e in main if e["type"] == "SubagentReturn")
sub_ret["cause"] = sub_call["id"]
sub_root = next(e for e in sub if e["type"] != "HarnessContext" and not e["cause"])
sub_root["cause"] = sub_call["id"]  # the graft
root = next(e for e in main if e["type"] != "HarnessContext" and not e["cause"])
T0 = root["t"]
kids = {}
for e in allev:
    if e.get("cause"):
        kids.setdefault(e["cause"], []).append(e)
for k in kids.values():
    k.sort(key=lambda e: e["t"])
results = {e["toolUseId"]: e for e in allev if e["type"] == "ToolResult"}

def size(e):
    return 1 + sum(size(c) for c in kids.get(e["id"], []))

# ---- 06: the real trace -----------------------------------------------------
def trace_figure(W=1456, H=820, t_max=280, x0=150, x1=1400, y_main=270, y_bg=420, y_sub=610,
                 labels=True, title_block=None):
    sx = lambda t: x0 + (t - T0) / 1000 / t_max * (x1 - x0)
    pos = {}

    def place(e, y):
        pos[e["id"]] = (sx(e["t"]), y)
        ch = kids.get(e["id"], [])
        if e is sub_call:
            for c in ch:
                if c is sub_root: place(c, y_sub)
                elif c is sub_ret: place(c, y)
                elif c["type"] == "ToolResult": place(c, y + 46)
                else: place(c, y_bg)
            return
        ch = sorted(ch, key=size, reverse=True)
        for i, c in enumerate(ch):
            place(c, y + 46 * i)
    place(root, y_main)

    body = []
    if title_block:
        body.append(title_block)
    # lanes
    lanes = [(y_main, "main agent"), (y_bg, "main agent, while the subagent runs"), (y_sub, "fact-checker subagent")]
    for y, name in lanes:
        top = y - (100 if y == y_main else 34)
        if labels: body.append(f'<rect x="{x0-20}" y="{top}" width="{x1-x0+40}" height="{y + (82 if y == y_main else 62) - top}" rx="16" fill="#fff" stroke="{LINE}" stroke-width="2"/>')
        if labels:
            body.append(text(x0 - 4, y - 46 - (64 if y == y_main else 0), name.upper(), 14, MUTED, 700))
    # axis
    ya = H - 70
    body.append(f'<line x1="{x0}" y1="{ya}" x2="{x1+16}" y2="{ya}" stroke="{COCOA}" stroke-width="3" marker-end="url(#ck)"/>')
    for s in range(0, t_max + 1, 30):
        x = x0 + s / t_max * (x1 - x0)
        body.append(f'<line x1="{x:.1f}" y1="{ya-6}" x2="{x:.1f}" y2="{ya+6}" stroke="{COCOA}" stroke-width="2"/>')
        body.append(text(x, ya + 28, f"{s} s", 15, MUTED, anchor="middle"))
    # edges
    for e in allev:
        if not e.get("cause") or e["id"] not in pos or e["cause"] not in pos:
            continue
        (xa, ya_), (xb, yb) = pos[e["cause"]], pos[e["id"]]
        graft = e is sub_root
        col, w = (PINK_D, 4) if graft else (COCOA, 2)
        if abs(ya_ - yb) < 1:
            body.append(f'<line x1="{xa:.1f}" y1="{ya_}" x2="{xb:.1f}" y2="{yb}" stroke="{col}" stroke-width="{w}"/>')
        else:
            body.append(f'<path d="M{xa:.1f} {ya_} C{xa:.1f} {(ya_+yb)/2:.1f} {xb:.1f} {(ya_+yb)/2:.1f} {xb:.1f} {yb}" fill="none" stroke="{col}" stroke-width="{w}"/>')
    # call spans
    for e in allev:
        if e["id"] in pos and e["type"] in ("ToolCall", "SubagentCall"):
            end = sub_ret["t"] if e is sub_call else results.get(e.get("toolUseId"), e)["t"]
            x, y = pos[e["id"]]
            w = max(sx(end) - x, 3)
            body.append(f'<rect x="{x:.1f}" y="{y-9}" width="{w:.1f}" height="18" rx="6" fill="{FILL[e["type"]]}" stroke="{COCOA}" stroke-width="1.5" opacity="0.9"/>')
    # nodes
    for e in allev:
        if e["id"] in pos and e["type"] not in ("ToolCall", "SubagentCall"):
            x, y = pos[e["id"]]
            r = 9 if e["type"] in ("Prompt", "SubagentReturn") else 5
            body.append(f'<circle cx="{x:.1f}" cy="{y}" r="{r}" fill="{FILL[e["type"]]}" stroke="{COCOA}" stroke-width="2"/>')
    return body, pos, sx

def rel(e):
    return (e["t"] - T0) / 1000

def find(label_part, typ="ToolCall", pool=main):
    return [e for e in pool if e["type"] == typ and label_part in (e["label"] or "")]

body, pos, sx = trace_figure()
fetches = find("arxiv.org/html")
searches = [e for e in main if e["tool"] == "WebSearch"]
notes = [
    (root, "prompt", 0),
    (find("Search lat docs")[0], "search docs", 1),
    (searches[0], "2 web searches", 0),
    (fetches[0], f"fetch paper, {(results[fetches[0]['toolUseId']]['t']-fetches[0]['t'])/1000:.1f} s", 1),
    (fetches[1], f"fetch again, {(results[fetches[1]['toolUseId']]['t']-fetches[1]['t'])/1000:.1f} s", 0),
    (searches[2], "PROV-AGENT, OTel", 1),
    (sub_call, f"subagent call at {rel(sub_call):.1f} s", 0),
    (sub_ret, f"hand-back at {rel(sub_ret):.1f} s", 1),
]
for e, s_, row in notes:
    x, y = pos[e["id"]]
    ty = y - 40 - 30 * row
    body.append(f'<line x1="{x:.1f}" y1="{y-12}" x2="{x:.1f}" y2="{ty+6}" stroke="{GREY}" stroke-width="1.5"/>')
    body.append(text(x - 4, ty, s_, 15, INK, 700))
sx_, sy_ = pos[sub_root["id"]]
body.append(text(sx_ - 14, sy_ + 6, f"subagent root, {rel(sub_root):.1f} s", 15, INK, 700, anchor="end"))
gx, gy = pos[sub_root["id"]]
body.append(text(gx + 14, 520, "graft", 16, PINK_D, 700))
n_sub_calls = sum(1 for e in sub if e["type"] == "ToolCall" and e["tool"] == "Bash")
body.append(text(1140, 610 + 6, f"{n_sub_calls} shell calls to read and check the paper", 15, MUTED))
body.insert(0, text(60, 56, "THE FIRST FOUR AND A HALF MINUTES OF THE SESSION THAT WROTE THIS ARTICLE, AS A CTEG", 16, MUTED, 700))
# legend
lx, ly = 60, 92
for i, (name, typ) in enumerate([("prompt", "Prompt"), ("reasoning / message", "Thinking"), ("tool call (span)", "ToolCall"),
                                  ("result", "ToolResult"), ("subagent call (span)", "SubagentCall")]):
    x = lx + i * 250
    if "span" in name:
        body.append(f'<rect x="{x}" y="{ly-9}" width="34" height="18" rx="6" fill="{FILL[typ]}" stroke="{COCOA}" stroke-width="1.5"/>')
    else:
        body.append(f'<circle cx="{x+17}" cy="{ly}" r="8" fill="{FILL[typ]}" stroke="{COCOA}" stroke-width="2"/>')
    body.append(text(x + 46, ly + 6, name, 16))
open(f"{out}/06-real-trace.svg", "w").write(svg(1456, 820, "The real agent trace as a CTEG on a time axis", "\n".join(body)))

# ---- 01: cover ----------------------------------------------------------------
cover_title = "\n".join([
    text(60, 120, "Causal-temporal", 76, INK, 800),
    text(60, 204, "event graphs", 76, INK, 800),
    text(64, 262, "agent traces with time in the database · SPARQL on Tiramemsu", 26, MUTED),
])
body, pos, sx = trace_figure(W=1456, H=816, x0=120, x1=1380, y_main=400, y_bg=520, y_sub=640, labels=False,
                             title_block=cover_title)
open(f"{out}/01-cover.svg", "w").write(svg(1456, 816, "Causal-temporal event graphs on Tiramemsu", "\n".join(body)))

# ---- 05: raw log versus CTEG --------------------------------------------------
start = next(i for i, e in enumerate(main) if e["type"] == "ToolResult")
win = main[start:start + 16]
W, H = 1456, 860
b = [text(60, 56, "THE SAME 16 RECORDS, IN LOG ORDER: TWO PARENTHOOD RULES", 16, MUTED, 700)]
xs = {e["id"]: 110 + i * 82 for i, e in enumerate(win)}
win_ids = set(xs)

def node(e, x, y, faded=False):
    ctx = e["type"] == "HarnessContext"
    stroke = f'stroke="{GREY}" stroke-dasharray="5 4"' if ctx else f'stroke="{COCOA}"'
    op = ' opacity="0.45"' if faded else ""
    return f'<circle cx="{x}" cy="{y}" r="{16 if not ctx else 12}" fill="{FILL[e["type"]]}" {stroke} stroke-width="3"{op}/>'

short = {"ToolResult": "result", "ToolCall": "call", "HarnessContext": "context", "Thinking": "reason", "Message": "msg"}
# top: raw
yt = 230
b.append(f'<rect x="40" y="90" width="1376" height="300" rx="20" fill="#fff" stroke="{LINE}" stroke-width="3"/>')
b.append(text(70, 130, "Raw log: parent = the previous record the harness wrote (parentUuid)", 19, INK, 700))
back_seen = False
for e in win:
    p = by_id.get(e["parent"])
    if p and p["id"] in win_ids:
        xa, xb = xs[p["id"]], xs[e["id"]]
        bad = e["t"] <= p["t"]
        col = PINK_D if bad else COCOA
        y_off = 0 if xb - xa == 82 else -40
        if y_off:
            b.append(f'<path d="M{xa} {yt-16} C{xa} {yt-70} {xb} {yt-70} {xb} {yt-18}" fill="none" stroke="{col}" stroke-width="{4 if bad else 2.5}" marker-end="url(#{"pk" if bad else "ck"})"/>')
        else:
            b.append(f'<line x1="{xa+16}" y1="{yt}" x2="{xb-18}" y2="{yt}" stroke="{col}" stroke-width="{4 if bad else 2.5}" marker-end="url(#{"pk" if bad else "ck"})"/>')
        if bad:
            b.append(text((xa + xb) / 2, yt + (-58 if y_off else -14), "≤" if e["t"] == p["t"] else "←", 18, PINK_D, 700, anchor="middle"))
for e in win:
    b.append(node(e, xs[e["id"]], yt))
    b.append(text(xs[e["id"]], yt + 40, short.get(e["type"], e["type"]), 13, MUTED, anchor="middle"))
    b.append(text(xs[e["id"]], yt + 60, f"{rel(e):.3f}", 13, INK, anchor="middle", mono=True))
b.append(text(70, 356, "Red: the child is stamped no later than its parent. In the whole trace: 77 such links, and 180 of 204 events", 17, INK))
b.append(text(70, 380, "cannot be reached by any chain that moves forward in time.", 17, INK))
# bottom: CTEG
yb = 560
b.append(f'<rect x="40" y="420" width="1376" height="400" rx="20" fill="#fff" stroke="{LINE}" stroke-width="3"/>')
b.append(text(70, 460, "CTEG: harness context causes nothing; parent = nearest logged ancestor that is not context", 19, INK, 700))
for e in win:
    if e["type"] == "HarnessContext":
        continue
    c = by_id.get(e["cause"])
    if c and c["id"] in win_ids:
        xa, xb = xs[c["id"]], xs[e["id"]]
        b.append(f'<path d="M{xa} {yb-16} C{xa} {yb-80} {xb} {yb-80} {xb} {yb-18}" fill="none" stroke="{COCOA}" stroke-width="3" marker-end="url(#ck)"/>')
for e in win:
    y = yb if e["type"] != "HarnessContext" else yb + 90
    b.append(node(e, xs[e["id"]], y, faded=e["type"] == "HarnessContext"))
    if e["type"] != "HarnessContext":
        b.append(text(xs[e["id"]], yb + 40, short.get(e["type"], e["type"]), 13, MUTED, anchor="middle"))
        b.append(text(xs[e["id"]], yb + 60, f"{rel(e):.3f}", 13, INK, anchor="middle", mono=True))
b.append(text(70, yb + 130, "harness context, kept as evidence but not a cause", 15, MUTED))
b.append(text(70, 760, "Every causal link goes forward. In the whole trace: 0 backward links, 0 of 89 events cut off.", 17, INK))
b.append(text(70, 786, "A second rule moves the subagent's hand-back under the call that started it.", 17, MUTED))
open(f"{out}/05-raw-vs-cteg.svg", "w").write(svg(W, H, "Raw log versus CTEG", "\n".join(b)))

# ---- 08: beyond single parenthood --------------------------------------------
sa, sb = searches[0], searches[1]
ra, rb = results[sa["toolUseId"]], results[sb["toolUseId"]]
nxt = next(e for e in main if e["cause"] == rb["id"])
W, H = 1456, 700
b = [text(60, 56, "A REAL PARALLEL FORK: TWO WEB SEARCHES, ONE NEXT STEP", 16, MUTED, 700)]
x0, x1, t_lo, t_hi = 140, 1360, sa["t"] - 2000, nxt["t"] + 1500
X = lambda e: x0 + (e["t"] - t_lo) / (t_hi - t_lo) * (x1 - x0)
P = {sa["id"]: (X(sa), 220), sb["id"]: (X(sb), 400), ra["id"]: (X(ra), 220), rb["id"]: (X(rb), 400), nxt["id"]: (X(nxt), 400)}
def box(e, label):
    x, y = P[e["id"]]
    return (f'<rect x="{x-70:.1f}" y="{y-26}" width="140" height="52" rx="12" fill="{FILL[e["type"]]}" stroke="{COCOA}" stroke-width="3"/>'
            + text(x, y + 6, label, 17, INK, 700, anchor="middle")
            + text(x, y + (52 if y > 300 else -38), f"{rel(e):.3f} s", 15, MUTED, anchor="middle", mono=True))
def arrow(a, c, dashed=False):
    (xa, ya), (xc, yc) = P[a["id"]], P[c["id"]]
    col = PINK_D if dashed else COCOA
    dash = ' stroke-dasharray="10 7"' if dashed else ""
    if ya == yc:
        return f'<line x1="{xa+72:.1f}" y1="{ya}" x2="{xc-74:.1f}" y2="{yc}" stroke="{col}" stroke-width="4"{dash} marker-end="url(#{"pk" if dashed else "ck"})"/>'
    return f'<path d="M{xa-50:.1f} {ya+28} C{xa-50:.1f} {(ya+yc)/2:.1f} {xc-90:.1f} {(ya+yc)/2:.1f} {xc-74:.1f} {yc-8}" fill="none" stroke="{col}" stroke-width="4"{dash} marker-end="url(#{"pk" if dashed else "ck"})"/>'
b += [arrow(sa, ra), arrow(sa, sb), arrow(sb, rb), arrow(rb, nxt), arrow(ra, nxt, dashed=True)]
b += [box(sa, "search A"), box(sb, "search B"), box(ra, "result A"), box(rb, "result B"), box(nxt, "fetch paper")]
b.append(text(P[ra["id"]][0] + 90, 300, "v:consumed", 18, PINK_D, 700))
b.append(f'<rect x="60" y="520" width="1336" height="140" rx="20" fill="#fff" stroke="{LINE}" stroke-width="3"/>')
b.append(text(90, 560, "Solid: v:caused, the single-parent CTEG. The harness logs the second parallel call under the first.", 18, INK))
b.append(text(90, 592, "Dashed: v:consumed. The next step read both results, but only result B can be its parent.", 18, INK))
b.append(text(90, 624, "Both are statements in one graph, so the tree keeps its guarantees and the extra dependency is not lost.", 18, MUTED))
open(f"{out}/08-beyond-single-parent.svg", "w").write(svg(W, H, "Beyond single parenthood", "\n".join(b)))
print("ok", sa["label"], sb["label"], rel(ra), rel(rb), nxt["label"])
