# Writes the 20 arcade and carnival models of the props pass (docs/PROPS_PASS.md) and the 46 models of the regions pass
# (docs/REGIONS_PASS.md, the second half of this file): cubes only, one shared palette
# picture, hidden faces left untextured. Run: python3 tools/props/props_pass_models.py src/bin/last_route/assets
import json, zlib, struct, base64, uuid, os, sys
OUT = sys.argv[1]
PAL = [("black","14141a"),("dark","2a2c36"),("grey","6b6f7a"),("metal","a9afb8"),("white","f2f0e6"),("cream","e8d9b0"),
 ("red","d9342b"),("crimson","8f1622"),("orange","f08a24"),("amber","f5b83d"),("yellow","f7dc4a"),("gold","c99a2e"),
 ("lime","8fd93a"),("green","2f9e4a"),("dkgreen","1f5a34"),("teal","1fa7a0"),("cyan","4fd8f0"),("sky","7fb8f5"),
 ("blue","2a5bd0"),("navy","1a2a66"),("violet","6a3fc8"),("purple","3e1f73"),("magenta","e23fb5"),("pink","f59ac4"),
 ("hotpink","ff5fa2"),("brown","6b4226"),("wood","a3703f"),("tan","c9a070"),("water","2f8fd8"),("waterlt","6fc3f0"),
 ("screen","0c1a2a"),("glass","9fd0e0")]
IDX = {n:i for i,(n,_) in enumerate(PAL)}
CELL = 8
W, H = CELL*len(PAL), CELL
def png():
    rows = b""
    line = b"".join(bytes.fromhex(h)+b"\xff" for _,h in PAL for _ in range(CELL))
    for _ in range(H): rows += b"\x00" + line
    def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t+d) & 0xffffffff)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b"")
SRC = "data:image/png;base64," + base64.b64encode(png()).decode()
NS = uuid.UUID("6f1d2c1e-0000-5000-8000-00000000a11c")
def r(x): return round(float(x), 3)
def model(path, cubes):
    name = os.path.basename(path)
    els, outl, seen = [], [], {}
    for (n, a, b, col) in cubes:
        k = seen.get(n, 0); seen[n] = k + 1
        en = n if k == 0 else "%s_%d" % (n, k)
        f = [r(min(a[i], b[i])) for i in range(3)]; t = [r(max(a[i], b[i])) for i in range(3)]
        assert all(t[i] > f[i] for i in range(3)), (name, en, f, t)
        c = IDX[col]; uv = [c*CELL+2.5, 2.5, c*CELL+5.5, 5.5]
        u = str(uuid.uuid5(NS, name + "/" + en))
        els.append({"name": en, "type": "cube", "uuid": u, "box_uv": False, "rescale": False, "locked": False,
            "from": f, "to": t, "autouv": 0, "color": c % 8, "origin": [0, 0, 0], "rotation": [0, 0, 0],
            "faces": {d: {"uv": uv, "texture": 0} for d in ["north", "south", "east", "west", "up", "down"]}})
        outl.append(u)
    # a face that lies inside or flat against another cube, or on the floor, is never seen: no texture, so it is not drawn
    AX = {"west": (0, 0), "east": (0, 1), "down": (1, 0), "up": (1, 1), "north": (2, 0), "south": (2, 1)}
    tris = 0
    for e in els:
        for d, (ax, hi) in AX.items():
            p = e["to"][ax] if hi else e["from"][ax]
            o = [i for i in range(3) if i != ax]
            hidden = d == "down" and p <= 0.0
            for b in els:
                if hidden: break
                if b is e: continue
                inside = (b["from"][ax] <= p < b["to"][ax]) if hi else (b["from"][ax] < p <= b["to"][ax])
                if inside and all(b["from"][i] <= e["from"][i] and e["to"][i] <= b["to"][i] for i in o): hidden = True
            if hidden: e["faces"][d] = {"uv": [0, 0, 0, 0], "texture": None}
            else: tris += 2
    j = {"meta": {"format_version": "4.10", "model_format": "free", "box_uv": False}, "name": name, "model_identifier": "",
        "visible_box": [1, 1, 0], "variable_placeholders": "", "variable_placeholder_buttons": [], "timeline_setups": [],
        "unhandled_root_fields": {}, "resolution": {"width": W, "height": H}, "elements": els, "outliner": outl,
        "textures": [{"path": "", "name": "props_pass_palette.png", "folder": "", "namespace": "", "id": "0", "group": "",
            "width": W, "height": H, "uv_width": W, "uv_height": H, "particle": False, "use_as_default": False,
            "layers_enabled": False, "sync_to_project": "", "render_mode": "default", "render_sides": "auto",
            "frame_time": 1, "frame_order_type": "loop", "frame_order": "", "frame_interpolate": False, "visible": True,
            "internal": True, "saved": True, "uuid": str(uuid.uuid5(NS, "props_pass_palette.png")),
            "relative_path": "props_pass_palette.png", "source": SRC}]}
    full = os.path.join(OUT, path + ".bbmodel")
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w") as fh: fh.write(json.dumps(j, separators=(",", ":")) + "\n")
    mn = [min(e["from"][i] for e in els) for i in range(3)]; mx = [max(e["to"][i] for e in els) for i in range(3)]
    print("%-46s %3d cubes %4d tris %6d bytes  %s %s" % (path, len(els), tris, os.path.getsize(full), mn, mx))
def box(n, x0, y0, z0, x1, y1, z1, col): return (n, (x0, y0, z0), (x1, y1, z1), col)

# ---------------- arcade cabinets (bounds x +-4.8, y 0..29.1, z -4.8..7.0; front is +z) ----------------
def glyphs(kind, a, b):
    g = []; z0, z1 = 3.8, 3.92
    def px(x0, y0, x1, y1, c): g.append(box("ScreenGlyph", x0, y0, z0, x1, y1, z1, c))
    if kind == "invaders":
        for row in range(2):
            for i in range(4): px(-2.3 + i*1.25, 22.9 - row*1.1, -1.6 + i*1.25, 23.5 - row*1.1, a if (i + row) % 2 == 0 else b)
        px(-0.5, 19.9, 0.5, 20.4, b); px(-0.1, 20.4, 0.1, 21.2, a)
    elif kind == "road":
        for i in range(4):
            w = 2.4 - i*0.55; y = 19.7 + i*1.05
            px(-w, y, -w + 0.25, y + 0.9, a); px(w - 0.25, y, w, y + 0.9, a)
            px(-0.1, y + 0.2, 0.1, y + 0.7, b)
        px(-0.6, 19.9, 0.6, 20.7, b)
    elif kind == "maze":
        px(-2.4, 23.6, 2.4, 23.9, a); px(-2.4, 19.9, 2.4, 20.2, a); px(-2.4, 20.2, -2.1, 23.6, a); px(2.1, 20.2, 2.4, 23.6, a)
        px(-1.2, 21.0, -0.9, 22.9, a); px(-0.9, 21.0, 0.6, 21.3, a); px(0.9, 21.9, 1.2, 23.6, a)
        px(-0.3, 22.0, 0.3, 22.6, b); px(1.4, 20.6, 1.8, 21.0, b); px(-1.8, 20.6, -1.5, 20.9, b)
    elif kind == "bars":
        for i, h in enumerate([1.2, 2.6, 3.8, 2.0, 3.1, 1.5, 2.8]):
            px(-2.45 + i*0.72, 19.8, -1.95 + i*0.72, 19.8 + h, a if i % 2 == 0 else b)
    elif kind == "crosshair":
        px(-0.1, 20.4, 0.1, 23.6, a); px(-1.6, 21.9, 1.6, 22.1, a)
        px(-1.9, 23.2, -1.2, 23.4, b); px(1.2, 23.2, 1.9, 23.4, b); px(-1.9, 20.6, -1.2, 20.8, b); px(1.2, 20.6, 1.9, 20.8, b)
        px(-1.9, 20.6, -1.7, 21.3, b); px(1.7, 20.6, 1.9, 21.3, b); px(-1.9, 22.7, -1.7, 23.4, b); px(1.7, 22.7, 1.9, 23.4, b)
    elif kind == "pong":
        px(-2.3, 20.6, -2.0, 22.2, a); px(2.0, 21.6, 2.3, 23.2, a); px(0.5, 21.6, 0.9, 22.0, b)
        for i in range(5): px(-0.05, 19.9 + i*0.9, 0.05, 20.4 + i*0.9, b)
    elif kind == "blocks":
        px(-2.4, 19.8, -0.8, 20.5, a); px(-0.8, 19.8, 0.0, 21.2, b); px(0.0, 19.8, 2.4, 20.5, a); px(1.6, 20.5, 2.4, 21.9, b)
        px(-2.4, 20.5, -1.6, 21.2, b); px(-0.4, 23.0, 0.4, 23.7, a); px(-0.4, 22.3, 1.2, 23.0, a)
    elif kind == "hearts":
        for cx, cy, c in [(-1.4, 22.6, a), (0.0, 21.4, b), (1.4, 22.6, a)]:
            px(cx - 0.6, cy, cx + 0.6, cy + 0.5, c); px(cx - 0.35, cy - 0.45, cx + 0.35, cy, c); px(cx - 0.6, cy + 0.5, cx - 0.1, cy + 0.8, c); px(cx + 0.1, cy + 0.5, cx + 0.6, cy + 0.8, c)
    return g
def cabinet(name, body, trim, marquee, face, screen, ga, gb, pattern, controls):
    c = [box("CabinetBase", -4.3, 0, -4.8, 4.3, 11.5, 4.8, body), box("CabinetUpper", -4.3, 11.4, -4.6, 4.3, 26.1, 3.4, body),
         box("Marquee", -4.8, 26.2, -4.2, 4.8, 29.1, 4.2, marquee), box("MarqueeFace", -3.7, 26.9, 4.2, 3.7, 28.5, 4.4, face),
         box("MarqueeStripe", -3.1, 27.5, 4.4, 3.1, 27.9, 4.5, ga),
         box("EdgeTrim_L", -4.6, 1.3, -4.6, -4.2, 28.5, 4.8, trim), box("EdgeTrim_R", 4.2, 1.3, -4.6, 4.6, 28.5, 4.8, trim),
         box("ScreenRecess", -3.4, 18.7, 3.3, 3.4, 25.1, 3.7, "black"), box("ScreenGlass", -2.8, 19.4, 3.7, 2.8, 24.5, 3.8, screen),
         box("ControlDeck", -4.3, 13.0, 3.4, 4.3, 14.6, 7.0, "dark"), box("DeckLip", -4.3, 12.4, 6.4, 4.3, 13.0, 7.0, trim),
         box("CoinDoor", -2.6, 3.4, 4.8, 2.6, 8.8, 5.0, "dark"), box("CoinSlot", -0.4, 6.5, 5.0, 0.4, 7.9, 5.1, face),
         box("KickPlate", -4.3, 0, 4.8, 4.3, 1.6, 5.0, "black"), box("SideArt_L", -4.4, 15.0, -3.0, -4.3, 23.0, 2.0, ga), box("SideArt_R", 4.3, 15.0, -3.0, 4.4, 23.0, 2.0, ga)]
    c += glyphs(pattern, ga, gb)
    if controls == "stick":
        c += [box("JoystickStem", -2.2, 14.6, 5.0, -1.9, 15.9, 5.3, "metal"), box("JoystickTop", -2.6, 15.7, 4.7, -1.5, 16.6, 5.6, face),
              box("ActionButton", -0.1, 14.6, 4.9, 0.8, 15.0, 5.8, ga), box("ActionButton", 1.2, 14.6, 5.3, 2.0, 15.0, 6.1, gb), box("ActionButton", 2.4, 14.6, 4.9, 3.3, 15.0, 5.8, ga)]
    elif controls == "wheel":
        c += [box("WheelColumn", -0.5, 14.6, 4.2, 0.5, 16.2, 5.2, "dark"), box("WheelRim", -2.2, 16.0, 5.0, 2.2, 16.5, 5.5, face), box("WheelRim", -2.2, 18.6, 5.0, 2.2, 19.1, 5.5, face),
              box("WheelRim", -2.2, 16.5, 5.0, -1.7, 18.6, 5.5, face), box("WheelRim", 1.7, 16.5, 5.0, 2.2, 18.6, 5.5, face), box("WheelSpoke", -1.7, 17.3, 5.0, 1.7, 17.8, 5.4, "metal"),
              box("GearShift", 3.0, 14.6, 4.6, 3.4, 16.4, 5.0, "metal"), box("GearKnob", 2.8, 16.4, 4.4, 3.6, 17.1, 5.2, ga), box("Pedal", -1.5, 0, 5.0, -0.3, 1.2, 6.6, "metal"), box("Pedal", 0.3, 0, 5.0, 1.5, 1.2, 6.6, "metal")]
    elif controls == "guns":
        for s, col in [(-1, ga), (1, gb)]:
            x = s*2.4
            c += [box("GunHolster", x - 0.7, 14.6, 4.2, x + 0.7, 15.6, 6.4, "black"), box("GunBody", x - 0.4, 15.6, 4.0, x + 0.4, 16.5, 6.8, col),
                  box("GunGrip", x - 0.35, 14.9, 6.4, x + 0.35, 15.6, 6.95, col), box("GunCord", x - 0.1, 9.0, 4.8, x + 0.1, 13.0, 4.95, "black")]
        c += [box("StartButton", -0.5, 14.6, 5.0, 0.5, 15.0, 6.0, face)]
    elif controls == "ball":
        c += [box("Trackball", -0.9, 14.6, 4.5, 0.9, 15.5, 6.3, face), box("ActionButton", -3.2, 14.6, 4.9, -2.2, 15.0, 5.9, ga), box("ActionButton", 2.2, 14.6, 4.9, 3.2, 15.0, 5.9, gb)]
    model("props/maze/" + name, c)
cabinet("arcade_cabinet_lime", "dkgreen", "lime", "black", "lime", "screen", "lime", "white", "invaders", "stick")
cabinet("arcade_cabinet_sunset", "crimson", "orange", "purple", "amber", "navy", "orange", "yellow", "blocks", "stick")
cabinet("arcade_cabinet_mono", "white", "black", "black", "white", "black", "white", "grey", "pong", "ball")
cabinet("arcade_cabinet_crimson", "black", "red", "crimson", "gold", "screen", "red", "gold", "maze", "stick")
cabinet("arcade_cabinet_racer", "navy", "yellow", "blue", "yellow", "dark", "white", "yellow", "road", "wheel")
cabinet("arcade_cabinet_blaster", "dark", "orange", "dkgreen", "orange", "screen", "lime", "red", "crosshair", "guns")
cabinet("arcade_cabinet_magenta_gold", "purple", "gold", "magenta", "gold", "navy", "magenta", "gold", "hearts", "stick")
cabinet("arcade_cabinet_cyan_lime", "navy", "cyan", "teal", "lime", "screen", "cyan", "lime", "bars", "ball")

# ---------------- claw machines (x +-6.6, y 0..29.9, z -6.1..6.9) ----------------
def claw(name, body, trim, back, plush):
    c = [box("ClawBase", -6.6, 0, -6.1, 6.6, 11.0, 6.1, body), box("PrizeBed", -6.0, 11.0, -5.5, 6.0, 12.0, 5.5, trim),
         box("BackPanel", -6.0, 12.0, -5.9, 6.0, 27.0, -5.5, back), box("ClawTop", -6.6, 27.0, -6.1, 6.6, 29.9, 6.1, body),
         box("TopSign", -5.0, 27.6, 6.1, 5.0, 29.3, 6.3, trim), box("ControlPanel", -3.2, 11.0, 6.1, 3.2, 12.6, 6.9, "dark"),
         box("JoystickStem", -1.6, 12.6, 6.3, -1.3, 13.8, 6.6, "metal"), box("JoystickTop", -1.9, 13.6, 6.0, -1.0, 14.4, 6.9, trim), box("DropButton", 1.0, 12.6, 6.2, 1.9, 13.0, 6.8, "red"),
         box("PrizeChute", -5.6, 3.0, 6.1, -1.6, 7.2, 6.3, "black"), box("CoinPlate", 2.0, 4.0, 6.1, 5.0, 8.0, 6.3, "metal"), box("CoinSlot", 3.3, 5.4, 6.3, 3.7, 6.8, 6.4, "black"),
         box("ClawRail", -6.0, 26.2, -0.4, 6.0, 27.0, 0.4, "metal"), box("ClawRod", 1.2, 19.5, -0.25, 1.7, 26.2, 0.25, "metal"), box("ClawHead", 0.6, 18.3, -0.9, 2.3, 19.5, 0.9, "grey"),
         box("ClawFinger", 0.4, 16.6, -0.2, 0.8, 18.3, 0.2, "metal"), box("ClawFinger", 2.1, 16.6, -0.2, 2.5, 18.3, 0.2, "metal"), box("ClawFinger", 1.25, 16.6, 0.7, 1.65, 18.3, 1.1, "metal")]
    for x in (-6.6, 5.8):
        for z in (-6.1, 5.3): c.append(box("GlassPost", x, 12.0, z, x + 0.8, 27.0, z + 0.8, trim))
    spots = [(-4.2, -3.5), (-1.0, -3.8), (2.6, -3.2), (-3.0, 0.2), (0.6, -0.4), (3.6, 1.0), (-4.4, 3.0), (-0.8, 2.8), (2.4, 3.6)]
    for i, (x, z) in enumerate(spots):
        col = plush[i % len(plush)]; h = 2.2 + (i % 3)*0.5
        c.append(box("Plush", x - 1.2, 12.0, z - 1.1, x + 1.2, 12.0 + h, z + 1.1, col))
        c.append(box("PlushHead", x - 0.8, 12.0 + h, z - 0.7, x + 0.8, 13.3 + h, z + 0.7, col))
    model("props/maze/" + name, c)
claw("prize_claw_machine_pink", "hotpink", "white", "sky", ["yellow", "teal", "violet", "white", "orange"])
claw("prize_claw_machine_blue", "blue", "yellow", "navy", ["pink", "lime", "orange", "white", "red"])

# ---------------- ticket redeemer (a token changer variant; x +-6.6, y 0..29.9, z -6.1..6.9) ----------------
c = [box("RedeemerBody", -6.6, 0, -6.1, 6.6, 26.5, 6.1, "violet"), box("RedeemerHeader", -6.6, 26.5, -6.1, 6.6, 29.9, 6.3, "yellow"),
     box("HeaderSign", -5.0, 27.2, 6.3, 5.0, 29.2, 6.5, "red"), box("ScreenRecess", -4.6, 17.0, 6.1, 4.6, 24.6, 6.3, "black"), box("ScreenGlass", -4.0, 17.6, 6.3, 4.0, 24.0, 6.4, "screen"),
     box("TicketEater", -3.0, 12.4, 6.1, 3.0, 14.6, 6.9, "metal"), box("TicketSlot", -2.2, 13.2, 6.9, 2.2, 13.6, 6.95, "black"),
     box("ReceiptTray", -4.0, 5.0, 6.1, 4.0, 8.4, 6.9, "dark"), box("KickPlate", -6.6, 0, 6.1, 6.6, 1.8, 6.3, "black"),
     box("SideStripe", -6.7, 8.0, -4.0, -6.6, 22.0, 4.0, "yellow"), box("SideStripe", 6.6, 8.0, -4.0, 6.7, 22.0, 4.0, "yellow")]
for i, col in enumerate(["yellow", "lime", "cyan", "yellow", "white"]):
    c.append(box("ScreenDigit", -3.4 + i*1.45, 19.2, 6.4, -2.5 + i*1.45, 22.4 - (i % 2)*0.9, 6.5, col))
for i in range(5): c.append(box("TicketStrip", 1.4, 9.0 + i*0.7, 6.9 - 0.02*i, 2.6, 9.6 + i*0.7, 6.98, "amber" if i % 2 == 0 else "cream"))
model("props/maze/arcade_ticket_redeemer", c)

# ---------------- prize plush shelf (a prize counter variant; x +-9.2, y 0..29.9, z -6.1..6.9) ----------------
c = [box("CounterBase", -9.2, 0, -6.1, 9.2, 10.0, 6.1, "wood"), box("CounterTop", -9.2, 10.0, -6.1, 9.2, 11.0, 6.9, "cream"), box("CounterFront", -8.4, 1.5, 6.1, 8.4, 8.5, 6.3, "red"),
     box("BackWall", -9.2, 11.0, -6.1, 9.2, 29.9, -5.2, "navy"), box("ShelfLow", -9.2, 16.5, -5.2, 9.2, 17.2, -0.6, "wood"), box("ShelfHigh", -9.2, 22.8, -5.2, 9.2, 23.5, -0.6, "wood"),
     box("ShelfSide", -9.2, 11.0, -5.2, -8.6, 29.9, -0.6, "wood"), box("ShelfSide", 8.6, 11.0, -5.2, 9.2, 29.9, -0.6, "wood"), box("TopValance", -9.2, 28.0, -5.2, 9.2, 29.9, 0.2, "red"),
     box("TicketJar", 5.6, 11.0, 2.2, 7.6, 14.2, 4.2, "glass"), box("TicketJarLid", 5.4, 14.2, 2.0, 7.8, 14.8, 4.4, "red"), box("PrizeBell", -7.2, 11.0, 3.0, -5.6, 12.2, 4.6, "gold")]
plush = ["pink", "teal", "yellow", "violet", "orange", "white", "lime", "sky"]
n = 0
for y, xs in [(17.2, [-6.0, -2.0, 2.0, 6.0]), (23.5, [-4.4, 0.0, 4.4]), (11.0, [-2.6, 1.0])]:
    for x in xs:
        col = plush[n % len(plush)]; n += 1; h = 2.6 if y > 20 else 3.0
        z = -3.0 if y > 11.5 else 1.6
        c += [box("PlushBody", x - 1.2, y, z - 1.0, x + 1.2, y + h, z + 1.0, col), box("PlushHead", x - 0.9, y + h, z - 0.8, x + 0.9, y + h + 1.5, z + 0.8, col),
              box("PlushNose", x - 0.2, y + h + 0.4, z + 0.8, x + 0.2, y + h + 0.8, z + 0.95, "black")]
        if n % 2 == 0:
            c += [box("PlushEar", x - 0.9, y + h + 1.5, z - 0.3, x - 0.4, y + h + 2.0, z + 0.3, col), box("PlushEar", x + 0.4, y + h + 1.5, z - 0.3, x + 0.9, y + h + 2.0, z + 0.3, col)]
model("props/maze/prize_plush_shelf", c)

# ---------------- carnival stalls (x +-20, y 0..41.6, z -11.6..15.1) ----------------
def stall(back, post, header, aw_a, aw_b, deck):
    c = [box("StallFloor", -19.6, 0, -11.6, 19.6, 1.5, 11.6, deck), box("StallBack", -19.6, 1.5, -11.5, 19.6, 37.6, -9.0, back),
         box("StallSide", -19.6, 1.5, -9.0, -18.6, 30.0, 8.2, back), box("StallSide", 18.6, 1.5, -9.0, 19.6, 30.0, 8.2, back),
         box("StallPost", -18.9, 1.5, 8.2, -16.3, 41.6, 10.7, post), box("StallPost", 16.3, 1.5, 8.2, 18.9, 41.6, 10.7, post),
         box("StallHeader", -20, 34.7, 7.3, 20, 41.4, 11.3, header)]
    for i in range(8):
        c.append(box("Awning", -20 + i*5, 33.2 - 0.0, 11.3, -15 + i*5, 35.2, 15.1, aw_a if i % 2 == 0 else aw_b))
    return c
# shooting gallery
c = stall("navy", "red", "red", "red", "white", "wood")
c += [box("Counter", -16.3, 1.5, 7.6, 16.3, 13.5, 11.2, "wood"), box("CounterTop", -16.8, 13.5, 7.2, 16.8, 14.5, 11.6, "cream"),
      box("HeaderSign", -12, 36.2, 11.3, 12, 40.0, 11.6, "yellow"), box("TargetRail", -17.5, 15.5, -8.8, 17.5, 16.5, -7.6, "metal"), box("TargetRail", -17.5, 24.5, -8.8, 17.5, 25.5, -7.6, "metal"),
      box("BigTarget", -3.5, 28.0, -9.0, 3.5, 35.0, -8.6, "white"), box("BigTargetRing", -2.2, 29.3, -8.6, 2.2, 33.7, -8.4, "red"), box("BigTargetEye", -0.9, 30.6, -8.4, 0.9, 32.4, -8.2, "yellow")]
for i in range(4):
    x = -12 + i*8.0
    c += [box("TargetStem", x - 0.2, 16.5, -8.4, x + 0.2, 18.0, -8.0, "metal"), box("Target", x - 1.7, 18.0, -8.5, x + 1.7, 21.4, -8.1, "white"), box("TargetEye", x - 0.7, 19.0, -8.1, x + 0.7, 20.4, -7.95, "red")]
for i in range(3):
    x = -8.0 + i*8.0
    c += [box("DuckTarget", x - 1.6, 25.5, -8.5, x + 1.6, 27.6, -8.1, "yellow"), box("DuckTargetHead", x + 0.6, 27.6, -8.5, x + 1.8, 28.8, -8.1, "yellow"), box("DuckTargetBeak", x + 1.8, 27.9, -8.5, x + 2.4, 28.3, -8.1, "orange")]
for x in (-8.0, 8.0):
    c += [box("RifleBarrel", x - 0.3, 14.9, 4.6, x + 0.3, 15.5, 10.2, "dark"), box("RifleStock", x - 0.45, 14.5, 10.2, x + 0.45, 15.9, 13.4, "brown"), box("RifleRest", x - 0.8, 14.5, 7.6, x + 0.8, 14.9, 8.6, "metal")]
model("props/carnival/shooting_gallery", c)
# whack-a-mole
c = stall("purple", "yellow", "orange", "orange", "yellow", "wood")
c += [box("MoleTable", -14.0, 1.5, -3.0, 14.0, 12.0, 9.0, "green"), box("MoleTableTop", -14.4, 12.0, -3.4, 14.4, 13.0, 9.4, "lime"), box("TableSkirt", -14.0, 1.5, 9.0, 14.0, 10.0, 9.3, "yellow"),
      box("HeaderSign", -12, 36.2, 11.3, 12, 40.0, 11.6, "purple"), box("ScoreBoard", -10.0, 22.0, -9.0, 10.0, 32.0, -8.5, "black"), box("ScoreFrame", -10.6, 21.4, -9.0, 10.6, 22.0, -8.3, "yellow"), box("ScoreFrame", -10.6, 32.0, -9.0, 10.6, 32.6, -8.3, "yellow")]
for i, col in enumerate(["red", "amber", "lime", "cyan", "magenta"]):
    c.append(box("ScoreLamp", -5.8 + i*2.6, 28.4, -8.5, -4.6 + i*2.6, 29.6, -8.2, col))
for i, h in enumerate([2.6, 1.4, 3.2]):
    c.append(box("ScoreDigit", -3.6 + i*2.8, 23.4, -8.5, -1.9 + i*2.8, 24.2 + h, -8.3, "amber"))
ups = [3.4, 0.0, 1.6, 0.0, 2.8, 0.0]
for k in range(6):
    x = -9.0 + (k % 3)*9.0; z = 0.4 + (k // 3)*5.4
    c.append(box("MoleHole", x - 2.3, 13.0, z - 2.0, x + 2.3, 13.15, z + 2.0, "black"))
    if ups[k] > 0:
        c += [box("Mole", x - 1.6, 13.15, z - 1.4, x + 1.6, 13.15 + ups[k], z + 1.4, "brown"), box("MoleNose", x - 0.4, 12.6 + ups[k], z + 1.4, x + 0.4, 13.1 + ups[k], z + 1.7, "pink"),
              box("MoleEye", x - 1.0, 13.3 + ups[k]*0.75, z + 1.4, x - 0.6, 13.6 + ups[k]*0.75, z + 1.5, "white"), box("MoleEye", x + 0.6, 13.3 + ups[k]*0.75, z + 1.4, x + 1.0, 13.6 + ups[k]*0.75, z + 1.5, "white")]
c += [box("MalletHandle", 10.6, 13.0, 3.0, 11.4, 13.8, 9.2, "wood"), box("MalletHead", 9.4, 13.0, 0.6, 12.6, 15.6, 3.0, "red"), box("MalletCord", 11.0, 9.0, 9.3, 11.2, 13.0, 9.4, "black")]
model("props/carnival/whack_a_mole", c)
# balloon dart stall (a ring toss variant)
c = stall("cream", "blue", "blue", "blue", "yellow", "wood")
c += [box("Counter", -16.3, 1.5, 7.6, 16.3, 13.5, 11.2, "blue"), box("CounterTop", -16.8, 13.5, 7.2, 16.8, 14.5, 11.6, "yellow"), box("HeaderSign", -12, 36.2, 11.3, 12, 40.0, 11.6, "white"),
      box("BalloonBoard", -17.0, 12.0, -9.0, 17.0, 33.0, -8.4, "tan"), box("BoardFrame", -17.6, 11.4, -9.0, 17.6, 12.0, -8.0, "wood"), box("BoardFrame", -17.6, 33.0, -9.0, 17.6, 33.6, -8.0, "wood")]
cols = ["red", "yellow", "lime", "cyan", "magenta", "orange", "sky", "pink", "violet"]
k = 0
for row in range(3):
    for i in range(6):
        x = -13.5 + i*5.4 + (1.2 if row % 2 else -1.2); y = 14.2 + row*6.2
        if (row*6 + i) % 7 == 4:
            c.append(box("PoppedBalloon", x - 0.5, y + 1.6, -8.4, x + 0.5, y + 2.2, -8.2, cols[k % 9])); k += 1; continue
        c += [box("Balloon", x - 1.5, y, -8.4, x + 1.5, y + 3.6, -6.6, cols[k % 9]), box("BalloonKnot", x - 0.3, y - 0.5, -8.0, x + 0.3, y, -7.4, "white")] if (row + i) % 2 == 0 else [box("Balloon", x - 1.5, y, -8.4, x + 1.5, y + 3.6, -6.6, cols[k % 9])]; k += 1
for i, x in enumerate([-8.0, 6.4, 8.2]):
    c += [box("Dart", x - 0.15, 14.5, 8.4, x + 0.15, 14.8, 10.4, "metal"), box("DartFlight", x - 0.45, 14.5, 10.4, x + 0.45, 15.1, 11.0, cols[i])]
c += [box("PrizeBear", -15.0, 14.5, 8.0, -12.4, 17.6, 10.4, "pink"), box("PrizeBearHead", -14.6, 17.6, 8.3, -12.8, 19.4, 10.1, "pink")]
model("props/carnival/balloon_dart_stall", c)

# ---------------- duck pond (x,z +-13.1, y 0..11.8) ----------------
c = []
for i, (hx, hz) in enumerate([(13.1, 6.0), (6.0, 13.1), (10.6, 10.6)]):
    c += [box("TubWall", -hx, 0, -hz, hx, 8.4 if i < 2 else 8.3, hz, "red" if i < 2 else "white"), box("TubWater", -hx + 1.2, 8.4, -hz + 1.2, hx - 1.2, 8.9, hz - 1.2, "water")]
c += [box("TubRim", -13.1, 8.4, -6.0, -12.3, 9.4, 6.0, "yellow"), box("TubRim", 12.3, 8.4, -6.0, 13.1, 9.4, 6.0, "yellow"), box("TubRim", -6.0, 8.4, -13.1, 6.0, 9.4, -12.3, "yellow"), box("TubRim", -6.0, 8.4, 12.3, 6.0, 9.4, 13.1, "yellow"),
      box("TubBand", -13.2, 3.0, -6.0, 13.2, 4.6, 6.0, "white"), box("TubBand", -6.0, 3.0, -13.2, 6.0, 4.6, 13.2, "white"),
      box("Ripple", -3.0, 8.9, -0.6, 3.0, 8.96, 0.6, "waterlt"), box("Ripple", 4.0, 8.9, 5.0, 8.0, 8.96, 5.6, "waterlt"), box("Ripple", -8.4, 8.9, -6.0, -4.8, 8.96, -5.4, "waterlt")]
for i, (x, z, d) in enumerate([(-7.0, 2.0, 1), (-2.0, -6.5, 1), (3.5, -1.5, -1), (7.5, 4.0, 1), (0.5, 7.0, -1), (-6.5, -4.0, -1)]):
    col = "yellow" if i != 3 else "white"
    c += [box("Duck", x - 1.3, 8.9, z - 0.9, x + 1.3, 10.2, z + 0.9, col), box("DuckHead", x + d*0.5 - 0.6, 10.2, z - 0.6, x + d*0.5 + 0.6, 11.4, z + 0.6, col),
          box("DuckBeak", x + d*1.1 - 0.4 + (0.4 if d > 0 else -0.4)*0, 10.5, z - 0.3, x + d*1.1 + 0.4, 10.9, z + 0.3, "orange"), box("DuckHook", x - 0.15, 11.4, z - 0.15, x + 0.15, 11.8, z + 0.15, "metal")]
c += [box("FishingRod", -12.6, 9.4, -5.0, -12.0, 9.9, 5.0, "wood"), box("FishingRod", 12.0, 9.4, -4.0, 12.6, 9.9, 5.6, "wood")]
model("props/carnival/duck_pond", c)

# ---------------- circus pedestal, blue and gold (x,z +-13.1, y 0..11.8) ----------------
c = []
for hx, hz in [(13.1, 6.0), (6.0, 13.1), (10.6, 10.6)]:
    c += [box("PedestalBase", -hx, 0, -hz, hx, 2.0, hz, "gold"), box("PedestalBody", -hx + 2.2, 2.0, -hz + 2.2, hx - 2.2, 9.6, hz - 2.2, "blue"), box("PedestalTop", -hx + 0.6, 9.6, -hz + 0.6, hx - 0.6, 11.8, hz - 0.6, "gold")]
for s in (-1, 1):
    c += [box("StarBlock", s*10.9 - 0.25, 4.0, -1.6, s*10.9 + 0.25, 7.6, 1.6, "white"), box("StarBlock", -1.6, 4.0, s*10.9 - 0.25, 1.6, 7.6, s*10.9 + 0.25, "white"),
          box("StripeBlock", s*10.9 - 0.2, 2.0, -3.6, s*10.9 + 0.2, 9.6, -2.6, "yellow"), box("StripeBlock", s*10.9 - 0.2, 2.0, 2.6, s*10.9 + 0.2, 9.6, 3.6, "yellow"),
          box("StripeBlock", -3.6, 2.0, s*10.9 - 0.2, -2.6, 9.6, s*10.9 + 0.2, "yellow"), box("StripeBlock", 2.6, 2.0, s*10.9 - 0.2, 3.6, 9.6, s*10.9 + 0.2, "yellow")]
c += [box("TopDisc", -7.0, 11.8 - 0.3, -7.0, 7.0, 11.8, 7.0, "red")]
model("props/carnival/circus_pedestal_blue", c)

# ---------------- ticket booth, blue and gold (x +-13.8, y 0..37.3, z -9.2..11.4) ----------------
c = [box("BoothBase", -12.5, 0, -8.5, 12.5, 14.0, 8.5, "blue"), box("BoothBack", -12.5, 14.0, -8.5, 12.5, 30.0, -7.5, "blue"), box("BoothCounter", -11.0, 14.0, 8.5, 11.0, 15.0, 11.4, "cream"),
     box("BoothRoof", -13.8, 30.0, -9.2, 13.8, 33.0, 11.0, "gold"), box("BoothRoofCap", -9.0, 33.0, -6.0, 9.0, 35.4, 6.0, "blue"), box("BoothFinial", -1.0, 35.4, -1.0, 1.0, 37.3, 1.0, "gold"),
     box("BoothSign", -8.0, 30.6, 11.0, 8.0, 32.6, 11.4, "white"), box("WindowSill", -12.5, 14.0, 7.5, 12.5, 15.4, 8.5, "gold"), box("WindowGlass", -9.0, 15.4, 7.9, 9.0, 27.0, 8.1, "glass"),
     box("WindowTop", -12.5, 27.0, 7.5, 12.5, 30.0, 8.5, "blue"), box("SpeakHole", -1.2, 19.5, 8.1, 1.2, 21.9, 8.2, "metal"), box("TicketRoll", 5.0, 15.0, 9.0, 7.4, 17.4, 10.4, "amber"), box("TicketTail", 5.6, 15.0, 10.4, 6.8, 15.2, 11.3, "amber"),
     box("BoothDoor", 12.5, 1.0, -5.0, 12.7, 26.0, 2.0, "navy"), box("DoorKnob", 12.7, 12.0, 0.6, 13.1, 12.8, 1.4, "gold")]
for x in (-12.5, 11.3):
    for z in (-8.5, 7.3): c.append(box("BoothPost", x, 14.0, z, x + 1.2, 30.0, z + 1.2, "gold"))
for i in range(6): c.append(box("BoothStripe", -11.0 + i*4.0, 1.0, 8.5, -9.0 + i*4.0, 13.0, 8.7, "gold" if i % 2 == 0 else "white"))
for i in range(7): c.append(box("RoofBulb", -12.0 + i*4.0 - 0.5, 29.0, 10.4, -12.0 + i*4.0 + 0.5, 30.0, 11.2, "yellow" if i % 2 == 0 else "red"))
model("props/carnival/carnival_ticket_booth_blue", c)

# ---------------- cotton candy cart (a popcorn cart variant; x -11.4..16.6, y 0..29.8, z -6.6..6.9) ----------------
c = [box("CartBody", -10.0, 6.0, -6.0, 10.0, 16.0, 6.0, "hotpink"), box("CartTrim", -10.2, 14.6, -6.2, 10.2, 16.0, 6.2, "white"), box("CartPanel", -8.0, 8.0, 6.0, 8.0, 13.6, 6.2, "sky"),
     box("Canopy", -11.4, 26.0, -6.6, -3.8, 29.8, 6.9, "pink"), box("Canopy", -3.8, 26.0, -6.6, 3.8, 29.8, 6.9, "white"), box("Canopy", 3.8, 26.0, -6.6, 11.4, 29.8, 6.9, "pink"),
     box("CanopyFringe", -11.4, 25.2, 6.3, 11.4, 26.0, 6.9, "sky"), box("SpinnerBowl", -5.5, 16.0, -4.5, 5.5, 18.4, 4.5, "metal"), box("SpinnerHead", -1.0, 18.4, -1.0, 1.0, 19.6, 1.0, "dark"),
     box("FlossInBowl", -4.2, 18.4, -3.2, 4.2, 19.0, 3.2, "pink"), box("HandleArm", 10.0, 13.6, -5.4, 16.6, 14.6, -4.4, "metal"), box("HandleArm", 10.0, 13.6, 4.4, 16.6, 14.6, 5.4, "metal"), box("HandleBar", 15.6, 13.6, -4.4, 16.6, 14.6, 4.4, "white"),
     box("Axle", -7.0, 2.6, -6.4, -5.0, 3.6, 6.4, "dark"), box("FrontLeg", 7.4, 0, -5.0, 8.6, 6.0, -3.8, "metal"), box("FrontLeg", 7.4, 0, 3.8, 8.6, 6.0, 5.0, "metal")]
for z0, z1 in [(-6.6, -5.8), (5.8, 6.6)]:
    c += [box("Wheel", -9.2, 0.8, z0, -2.8, 5.4, z1, "dark"), box("Wheel", -8.2, 0, z0, -3.8, 6.2, z1, "dark"), box("WheelHub", -6.8, 2.2, z0 - 0.0, -5.2, 4.0, z1 + 0.25 if z1 > 0 else z1, "white") if z1 > 0 else box("WheelHub", -6.8, 2.2, z0 - 0.25, -5.2, 4.0, z1, "white")]
for x in (-10.0, 9.2):
    for z in (-6.0, 5.2): c.append(box("CanopyPost", x, 16.0, z, x + 0.8, 26.0, z + 0.8, "white"))
for i, (x, z, col) in enumerate([(-8.0, 3.6, "pink"), (-8.0, 0.0, "sky"), (-8.0, -3.6, "pink"), (7.6, -3.4, "sky"), (7.6, 3.4, "hotpink")]):
    c += [box("FlossStick", x - 0.15, 16.0, z - 0.15, x + 0.15, 20.4, z + 0.15, "cream"), box("FlossPuff", x - 1.3, 20.4, z - 1.3, x + 1.3, 23.6, z + 1.3, col), box("FlossPuffTop", x - 0.8, 23.6, z - 0.8, x + 0.8, 24.6, z + 0.8, col)]
model("props/carnival/cotton_candy_cart", c)

# ---------------- high striker, blue (a strength tester variant; x -7.4..13.9, y 0..45.9, z -6.7..5.9) ----------------
c = [box("StrikerBase", -7.4, 0, -6.7, 7.4, 2.0, 5.9, "navy"), box("StrikerTower", -2.6, 2.0, -2.0, 2.6, 41.0, 2.0, "blue"), box("TowerCap", -3.4, 41.0, -2.6, 3.4, 42.4, 2.6, "gold"),
     box("Bell", -1.9, 42.4, -1.9, 1.9, 45.0, 1.9, "gold"), box("BellKnob", -0.6, 45.0, -0.6, 0.6, 45.9, 0.6, "gold"), box("ScaleStrip", -1.1, 5.0, 2.0, 1.1, 40.0, 2.25, "white"),
     box("Puck", -0.9, 14.0, 2.25, 0.9, 15.6, 2.7, "red"), box("PuckRail", -0.15, 5.0, 2.25, 0.15, 40.0, 2.4, "metal"), box("LeverPad", 4.0, 2.0, -2.2, 9.6, 3.4, 2.2, "red"), box("LeverPivot", 2.6, 2.0, -1.0, 4.0, 3.0, 1.0, "metal"),
     box("MalletHandle", 11.6, 0, -0.5, 12.6, 14.0, 0.5, "wood"), box("MalletHead", 10.3, 14.0, -1.6, 13.9, 17.6, 1.6, "brown"), box("MalletBand", 10.2, 15.2, -1.7, 14.0 - 0.1, 16.4, 1.7, "gold"),
     box("TowerSide", -3.0, 2.0, -2.2, -2.6, 41.0, 2.2, "gold"), box("TowerSide", 2.6, 2.0, -2.2, 3.0, 41.0, 2.2, "gold")]
for i, col in enumerate(["lime", "lime", "yellow", "yellow", "orange", "orange", "red"]):
    c.append(box("ScaleMark", -1.1, 8.0 + i*4.6, 2.25, -0.3, 9.0 + i*4.6, 2.35, col))
for i in range(4):
    c.append(box("TowerBulb", -3.3, 8.0 + i*9.0, 1.4, -2.9 + 0.0, 9.2 + i*9.0, 2.5, "yellow")); c.append(box("TowerBulb", 2.9, 8.0 + i*9.0, 1.4, 3.3, 9.2 + i*9.0, 2.5, "yellow"))
model("props/carnival/strength_tester_blue", c)

# =====================================================================================================================
# The regions pass (docs/REGIONS_PASS.md): 46 models for the office, the mansion, clutter and the three seasonal
# regions. Sizes are in metres here (16 model units a metre, drawn at scale 1); x and z are centred on the prop, the
# back is -z (the wall side) and the floor is y 0. `REGION_PASS_SIZES` at the end prints each model's box.
# =====================================================================================================================
REGION_PASS = []
def C(n, cx, cz, w, d, y0, y1, col):
    return box(n, (cx - w/2)*16, y0*16, (cz - d/2)*16, (cx + w/2)*16, y1*16, (cz + d/2)*16, col)
def rmodel(folder, name, cubes):
    # never write over a model this script did not make (the Godot originals share these folders)
    full = os.path.join(OUT, "props", folder, name + ".bbmodel")
    assert not os.path.exists(full) or "props_pass_palette.png" in open(full).read(), full + " is not a props-pass model"
    model("props/%s/%s" % (folder, name), cubes)
    xs = [v for c in cubes for v in (c[1][0], c[2][0])]; ys = [v for c in cubes for v in (c[1][1], c[2][1])]; zs = [v for c in cubes for v in (c[1][2], c[2][2])]
    assert min(ys) == 0, (name, min(ys))
    # the collider box is centred on the prop: twice the furthest reach on x and z
    REGION_PASS.append((folder, name, [round(2*max(-min(xs), max(xs))/16, 2), round(max(ys)/16, 2), round(2*max(-min(zs), max(zs))/16, 2)]))
def legs(n, w, d, y0, y1, t, col):
    return [C(n, sx*(w/2 - t/2), sz*(d/2 - t/2), t, t, y0, y1, col) for sx in (-1, 1) for sz in (-1, 1)]

# ---------------- office ----------------
c = [C("Foot", -0.8, 0, 0.08, 0.5, 0, 0.06, "dark"), C("Foot", 0.8, 0, 0.08, 0.5, 0, 0.06, "dark"),
     C("Post", -0.8, 0, 0.06, 0.06, 0.06, 1.9, "metal"), C("Post", 0.8, 0, 0.06, 0.06, 0.06, 1.9, "metal"),
     C("Board", 0, 0, 1.54, 0.04, 0.85, 1.85, "white"), C("FrameTop", 0, 0, 1.6, 0.06, 1.85, 1.9, "metal"), C("FrameBottom", 0, 0, 1.6, 0.06, 0.8, 0.85, "metal"),
     C("Tray", 0, 0.06, 1.2, 0.08, 0.76, 0.8, "grey"), C("Marker", -0.3, 0.07, 0.12, 0.03, 0.8, 0.83, "red"), C("Marker", 0.1, 0.07, 0.12, 0.03, 0.8, 0.83, "blue"),
     C("Scribble", -0.35, 0.025, 0.5, 0.01, 1.5, 1.53, "blue"), C("Scribble", -0.25, 0.025, 0.7, 0.01, 1.35, 1.38, "blue"), C("Scribble", 0.35, 0.025, 0.3, 0.01, 1.1, 1.4, "red"),
     C("Chart", 0.3, 0.025, 0.05, 0.01, 1.45, 1.7, "green"), C("Chart", 0.42, 0.025, 0.05, 0.01, 1.45, 1.62, "green")]
rmodel("maze", "office_whiteboard", c)
c = [C("Counter", 0, 0, 1.6, 0.6, 0, 0.86, "white"), C("CounterTop", 0, 0, 1.6, 0.6, 0.86, 0.9, "grey"), C("KickPlate", 0, 0.301, 1.6, 0.004, 0, 0.1, "dark"),
     C("CupboardDoor", -0.4, 0.302, 0.7, 0.006, 0.14, 0.82, "cream"), C("CupboardDoor", 0.4, 0.302, 0.7, 0.006, 0.14, 0.82, "cream"),
     C("DoorPull", -0.1, 0.306, 0.03, 0.01, 0.4, 0.6, "metal"), C("DoorPull", 0.1, 0.306, 0.03, 0.01, 0.4, 0.6, "metal"),
     C("CoffeeMachine", -0.5, -0.05, 0.3, 0.34, 0.9, 1.3, "black"), C("CoffeeTop", -0.5, -0.05, 0.26, 0.3, 1.3, 1.34, "metal"), C("CoffeePot", -0.5, 0.14, 0.14, 0.12, 0.92, 1.08, "glass"),
     C("Microwave", 0.4, -0.05, 0.5, 0.36, 0.9, 1.18, "metal"), C("MicrowaveDoor", 0.33, 0.132, 0.32, 0.006, 0.94, 1.14, "black"), C("MicrowavePanel", 0.58, 0.132, 0.1, 0.006, 0.94, 1.14, "dark"),
     C("Mug", -0.12, 0.1, 0.08, 0.08, 0.9, 1.0, "red"), C("Mug", 0.0, 0.16, 0.08, 0.08, 0.9, 1.0, "sky"), C("Backsplash", 0, -0.29, 1.6, 0.02, 0.9, 1.5, "cream")]
rmodel("maze", "office_coffee_station", c)

# ---------------- mansion ----------------
def portrait(name, frame, coat, back):
    c = [C("Plinth", 0, 0, 1.0, 0.12, 0, 0.1, "brown"), C("Stand", -0.4, 0, 0.06, 0.08, 0.1, 0.9, "brown"), C("Stand", 0.4, 0, 0.06, 0.08, 0.1, 0.9, "brown"),
         C("FrameBottom", 0, 0, 1.0, 0.12, 0.9, 1.0, frame), C("FrameTop", 0, 0, 1.0, 0.12, 1.9, 2.0, frame), C("FrameSide", -0.45, 0, 0.1, 0.12, 1.0, 1.9, frame),
         C("FrameSide", 0.45, 0, 0.1, 0.12, 1.0, 1.9, frame), C("Canvas", 0, -0.01, 0.8, 0.06, 1.0, 1.9, back), C("Shoulders", 0, 0.022, 0.5, 0.006, 1.0, 1.35, coat),
         C("Collar", 0, 0.026, 0.14, 0.004, 1.25, 1.35, "white"), C("Face", 0, 0.022, 0.24, 0.006, 1.35, 1.66, "tan"), C("Hair", 0, 0.026, 0.28, 0.004, 1.6, 1.74, "black"),
         C("Eye", -0.06, 0.026, 0.04, 0.004, 1.5, 1.53, "black"), C("Eye", 0.06, 0.026, 0.04, 0.004, 1.5, 1.53, "black")]
    rmodel("mansion", name, c)
portrait("mansion_portrait_frame", "gold", "crimson", "dkgreen")
portrait("mansion_portrait_frame_navy", "brown", "navy", "dark")
c = [C("Foot", 0, 0, 0.5, 0.5, 0, 0.06, "gold"), C("Stem", 0, 0, 0.08, 0.08, 0.06, 1.2, "gold"), C("Knop", 0, 0, 0.16, 0.16, 0.6, 0.7, "gold"), C("Arm", 0, 0, 0.5, 0.05, 1.2, 1.25, "gold"),
     C("Arm", 0, 0, 0.05, 0.5, 1.2, 1.25, "gold")]
for (x, z) in [(-0.22, 0), (0.22, 0), (0, -0.22), (0, 0.22), (0, 0)]:
    top = 1.5 if (x, z) == (0, 0) else 1.42
    c += [C("Cup", x, z, 0.09, 0.09, 1.25, 1.29, "gold"), C("Candle", x, z, 0.05, 0.05, 1.29, top, "cream"), C("CandleFlame", x, z, 0.03, 0.03, top, top + 0.1, "amber")]
rmodel("mansion", "candelabra", c)
c = [C("ArmoireBase", 0, 0, 1.4, 0.7, 0, 0.18, "brown"), C("ArmoireBody", 0, -0.02, 1.32, 0.62, 0.18, 2.05, "wood"), C("ArmoireCrown", 0, 0, 1.4, 0.7, 2.05, 2.2, "brown"),
     C("ArmoireDoor", -0.33, 0.292, 0.6, 0.006, 0.26, 1.98, "brown"), C("ArmoireDoor", 0.33, 0.292, 0.6, 0.006, 0.26, 1.98, "brown"),
     C("DoorPanel", -0.33, 0.296, 0.4, 0.004, 1.1, 1.85, "wood"), C("DoorPanel", 0.33, 0.296, 0.4, 0.004, 1.1, 1.85, "wood"), C("DoorPanel", -0.33, 0.296, 0.4, 0.004, 0.4, 0.95, "wood"),
     C("DoorPanel", 0.33, 0.296, 0.4, 0.004, 0.4, 0.95, "wood"), C("DoorKnob", -0.06, 0.3, 0.04, 0.012, 1.0, 1.06, "gold"), C("DoorKnob", 0.06, 0.3, 0.04, 0.012, 1.0, 1.06, "gold")]
rmodel("mansion", "mansion_armoire", c)
c = legs("StoolLeg", 0.56, 0.41, 0, 0.42, 0.06, "brown") + [C("StoolRail", 0, 0, 0.5, 0.35, 0.2, 0.24, "brown"), C("StoolSeat", 0, 0, 0.6, 0.45, 0.42, 0.5, "black"),
     C("StoolPad", 0, 0, 0.54, 0.39, 0.5, 0.55, "crimson")]
rmodel("mansion", "piano_stool", c)
c = [C("PedestalBase", 0, 0, 0.6, 0.6, 0, 0.1, "brown"), C("PedestalColumn", 0, 0, 0.44, 0.44, 0.1, 0.8, "wood"), C("PedestalTop", 0, 0, 0.6, 0.6, 0.8, 0.88, "brown"),
     C("Plaque", 0, 0.221, 0.24, 0.006, 0.4, 0.52, "gold"), C("OwlBody", 0, 0, 0.26, 0.22, 0.98, 1.3, "tan"), C("OwlHead", 0, 0.01, 0.3, 0.24, 1.3, 1.5, "brown"),
     C("OwlEye", -0.07, 0.131, 0.07, 0.006, 1.38, 1.45, "amber"), C("OwlEye", 0.07, 0.131, 0.07, 0.006, 1.38, 1.45, "amber"), C("OwlBeak", 0, 0.14, 0.04, 0.03, 1.33, 1.38, "gold"),
     C("OwlWing", -0.15, -0.01, 0.05, 0.18, 1.0, 1.26, "brown"), C("OwlWing", 0.15, -0.01, 0.05, 0.18, 1.0, 1.26, "brown"), C("Perch", 0, 0, 0.36, 0.06, 0.88, 0.98, "wood")]
rmodel("mansion", "taxidermy_pedestal", c)
c = [C("ArmourBase", 0, 0, 0.7, 0.6, 0, 0.1, "brown"), C("Boot", -0.14, 0.02, 0.16, 0.26, 0.1, 0.2, "grey"), C("Boot", 0.14, 0.02, 0.16, 0.26, 0.1, 0.2, "grey"),
     C("Greave", -0.14, 0, 0.15, 0.16, 0.2, 0.95, "metal"), C("Greave", 0.14, 0, 0.15, 0.16, 0.2, 0.95, "metal"), C("Tasset", 0, 0, 0.46, 0.24, 0.9, 1.05, "grey"),
     C("Cuirass", 0, 0, 0.5, 0.28, 1.05, 1.55, "metal"), C("Pauldron", -0.31, 0, 0.14, 0.26, 1.42, 1.6, "grey"), C("Pauldron", 0.31, 0, 0.14, 0.26, 1.42, 1.6, "grey"),
     C("Arm", -0.31, 0, 0.11, 0.14, 0.95, 1.42, "metal"), C("Arm", 0.31, 0, 0.11, 0.14, 0.95, 1.42, "metal"), C("Gorget", 0, 0, 0.2, 0.18, 1.55, 1.62, "grey"),
     C("Helm", 0, 0, 0.26, 0.28, 1.62, 1.92, "metal"), C("Visor", 0, 0.141, 0.2, 0.006, 1.74, 1.8, "black"), C("Plume", 0, -0.04, 0.06, 0.16, 1.92, 2.0, "red"),
     C("Halberd", 0.3, 0.2, 0.04, 0.04, 0.1, 1.9, "wood"), C("HalberdBlade", 0.3, 0.2, 0.04, 0.16, 1.66, 1.9, "metal")]
rmodel("mansion", "suit_of_armour", c)
def bookshelf(name, w, h, rows, d=0.45):
    c = [C("ShelfSide", -(w/2 - 0.03), 0, 0.06, d, 0, h, "brown"), C("ShelfSide", w/2 - 0.03, 0, 0.06, d, 0, h, "brown"), C("ShelfBack", 0, -(d/2 - 0.02), w - 0.12, 0.04, 0, h, "wood"),
         C("ShelfTop", 0, 0, w, d, h - 0.06, h, "brown"), C("ShelfPlinth", 0, 0, w - 0.12, d - 0.04, 0, 0.12, "brown")]
    cols = ["crimson", "navy", "dkgreen", "gold", "purple", "tan", "teal", "brown"]
    step = (h - 0.24) / rows
    for r_ in range(rows):
        y = 0.12 + r_*step
        c.append(C("ShelfBoard", 0, 0.01, w - 0.12, d - 0.06, y - 0.03, y, "brown"))
        x = -(w/2 - 0.1); i = r_*3
        while x + 0.2 < w/2 - 0.08:
            bw = 0.16 + 0.05*((i*7) % 3); bh = step*(0.62 + 0.1*((i*5) % 3))
            if (i + r_) % 5 != 4: c.append(C("BookBlock", x + bw/2, 0.04, bw, d - 0.2, y, y + bh, cols[(i*3 + r_) % len(cols)]))
            x += bw + 0.01; i += 1
    rmodel("mansion", name, c)
bookshelf("mansion_bookshelf_tall", 1.2, 2.3, 5)
bookshelf("mansion_bookshelf_low", 1.6, 1.1, 2)

# ---------------- clutter ----------------
c = [C("PaperStack", 0, 0, 0.3, 0.22, 0, 0.2, "white"), C("PaperStack", -0.08, 0.06, 0.3, 0.22, 0.2, 0.25, "cream"), C("LooseSheet", 0.12, -0.08, 0.26, 0.2, 0, 0.012, "white"),
     C("LooseSheet", -0.11, 0.09, 0.28, 0.2, 0, 0.01, "cream"), C("Folder", 0.02, 0.0, 0.3, 0.22, 0.2, 0.215, "amber")]
c = [C("FloorSheets", 0, 0, 0.5, 0.4, 0, 0.008, "white")] + c
rmodel("maze", "papers_stack", c)
c = [C("BoxLarge", -0.2, -0.1, 0.6, 0.6, 0, 0.5, "tan"), C("BoxTape", -0.2, -0.1, 0.08, 0.61, 0.5, 0.506, "cream"), C("BoxSmall", -0.18, -0.12, 0.44, 0.44, 0.5, 0.9, "wood"),
     C("BoxTape", -0.18, -0.12, 0.06, 0.45, 0.9, 0.906, "cream"), C("BoxSide", 0.3, 0.15, 0.4, 0.5, 0, 0.36, "tan"), C("BoxFlap", 0.3, 0.38, 0.4, 0.04, 0.36, 0.5, "wood"),
     C("BoxLabel", -0.2, 0.201, 0.2, 0.004, 0.2, 0.34, "white"), C("FloorFlap", 0, 0, 1.0, 0.8, 0, 0.006, "wood")]
rmodel("maze", "cardboard_boxes", c)
c = [C("PuddleCore", 0, 0, 0.9, 0.7, 0, 0.012, "water"), C("PuddleArm", 0.15, 0.1, 1.2, 0.4, 0, 0.01, "water"), C("PuddleArm", -0.1, -0.05, 0.5, 1.0, 0, 0.01, "water"),
     C("PuddleShine", -0.1, 0.05, 0.3, 0.12, 0.012, 0.016, "waterlt"), C("PuddleShine", 0.2, -0.1, 0.16, 0.08, 0.012, 0.016, "waterlt"),
     C("PuddleEdge", 0, 0, 1.2, 1.0, 0, 0.004, "water")]
rmodel("maze", "floor_puddle", c)
c = [C("TileSlab", 0, 0, 0.6, 0.6, 0, 0.04, "cream"), C("TileSlabTilted", 0.2, 0.18, 0.4, 0.34, 0.04, 0.12, "white"), C("TileChunk", -0.3, 0.26, 0.2, 0.16, 0, 0.05, "cream"),
     C("TileChunk", 0.3, -0.3, 0.16, 0.2, 0, 0.04, "white"), C("TileDust", 0, 0, 0.9, 0.9, 0, 0.006, "grey"), C("TileGridRail", -0.1, -0.2, 0.7, 0.03, 0.04, 0.07, "metal")]
rmodel("maze", "fallen_ceiling_tile", c)
c = [C("ConeBase", 0, 0, 0.4, 0.4, 0, 0.04, "black"), C("ConeTier", 0, 0, 0.28, 0.28, 0.04, 0.22, "orange"), C("ConeBand", 0, 0, 0.22, 0.22, 0.22, 0.34, "white"),
     C("ConeTier", 0, 0, 0.16, 0.16, 0.34, 0.5, "orange"), C("ConeBand", 0, 0, 0.11, 0.11, 0.5, 0.58, "white"), C("ConeTip", 0, 0, 0.07, 0.07, 0.58, 0.7, "orange")]
rmodel("maze", "traffic_cone", c)
c = [C("SignPanelFront", 0, 0.17, 0.4, 0.04, 0, 0.3, "yellow"), C("SignPanelFront", 0, 0.11, 0.4, 0.04, 0.3, 0.5, "yellow"), C("SignPanelBack", 0, -0.17, 0.4, 0.04, 0, 0.3, "yellow"),
     C("SignPanelBack", 0, -0.11, 0.4, 0.04, 0.3, 0.5, "yellow"), C("SignRidge", 0, 0, 0.4, 0.2, 0.5, 0.65, "yellow"), C("SignHandle", 0, 0, 0.16, 0.06, 0.55, 0.6, "black"),
     C("SignFigure", 0, 0.191, 0.08, 0.004, 0.1, 0.24, "black"), C("SignStripe", 0, 0.191, 0.3, 0.004, 0.04, 0.07, "black"), C("SignFoot", 0, 0, 0.4, 0.5, 0, 0.006, "black")]
rmodel("maze", "wet_floor_sign", c)
c = [C("SeatFallen", 0, 0.05, 0.46, 0.08, 0.02, 0.5, "wood"), C("BackFallen", 0, -0.2, 0.46, 0.5, 0.02, 0.1, "wood"), C("LegUp", -0.2, 0.3, 0.06, 0.44, 0.04, 0.1, "brown"),
     C("LegUp", 0.2, 0.3, 0.06, 0.44, 0.42, 0.48, "brown"), C("LegBroken", 0.3, -0.3, 0.2, 0.06, 0, 0.06, "brown"), C("LegBroken", -0.3, 0.35, 0.06, 0.2, 0, 0.06, "brown"),
     C("Splinters", 0, 0, 0.8, 0.9, 0, 0.006, "tan"), C("CrossRail", 0, 0.3, 0.46, 0.05, 0.5, 0.6, "brown")]
rmodel("maze", "broken_chair", c)
c = [C("Bucket", -0.05, 0.05, 0.36, 0.36, 0, 0.34, "yellow"), C("BucketRim", -0.05, 0.05, 0.4, 0.4, 0.34, 0.38, "amber"), C("BucketWater", -0.05, 0.05, 0.32, 0.32, 0.38, 0.384, "water"),
     C("Wringer", -0.05, -0.08, 0.3, 0.12, 0.38, 0.5, "red"), C("MopHandle", 0.14, -0.14, 0.04, 0.04, 0.3, 1.4, "wood"), C("MopHead", 0.14, -0.14, 0.2, 0.16, 0.06, 0.3, "white"),
     C("MopStrand", 0.14, -0.12, 0.22, 0.2, 0, 0.06, "cream"), C("FloorDrips", 0, 0, 0.5, 0.5, 0, 0.004, "waterlt")]
rmodel("maze", "bucket_and_mop", c)

# ---------------- festive ----------------
c = [C("TreePot", 0, 0, 0.5, 0.5, 0, 0.3, "crimson"), C("TreeTrunk", 0, 0, 0.14, 0.14, 0.3, 0.5, "brown")]
tiers = [(1.4, 0.5, 0.86), (1.16, 0.84, 1.2), (0.92, 1.18, 1.52), (0.68, 1.5, 1.82), (0.44, 1.8, 2.08), (0.2, 2.06, 2.24)]
for i, (w, y0, y1) in enumerate(tiers):
    c.append(C("TreeTier", 0, 0, w, w, y0, y1, "dkgreen" if i % 2 == 0 else "green"))
    if i < 5:
        cols = ["red", "gold", "sky", "white"]
        for k, (sx, sz) in enumerate([(-1, 1), (1, 1), (1, -1), (-1, -1)]):
            c.append(C("Bauble", sx*(w/2 - 0.04), sz*(w/2 - 0.04), 0.1, 0.1, y0 + 0.04, y0 + 0.14, cols[(i + k) % 4]))
        c.append(C("FairyLight", 0, w/2 + 0.002, w*0.5, 0.004, y0 + 0.2, y0 + 0.24, "yellow"))
c += [C("TreeStar", 0, 0, 0.22, 0.06, 2.24, 2.4, "yellow"), C("TreeStarArm", 0, 0, 0.06, 0.2, 2.28, 2.36, "yellow"), C("TreeSkirt", 0, 0, 1.0, 1.0, 0, 0.03, "white")]
rmodel("festive", "christmas_tree", c)
c = [C("PresentLarge", -0.22, -0.1, 0.5, 0.5, 0, 0.42, "red"), C("Ribbon", -0.22, -0.1, 0.08, 0.51, 0, 0.426, "gold"), C("Ribbon", -0.22, -0.1, 0.51, 0.08, 0, 0.426, "gold"),
     C("Bow", -0.22, -0.1, 0.2, 0.12, 0.426, 0.5, "gold"), C("PresentTall", 0.28, -0.14, 0.34, 0.34, 0, 0.6, "green"), C("Ribbon", 0.28, -0.14, 0.06, 0.35, 0, 0.606, "white"),
     C("PresentSmall", 0.2, 0.22, 0.3, 0.26, 0, 0.22, "blue"), C("Ribbon", 0.2, 0.22, 0.31, 0.05, 0, 0.226, "yellow"), C("PresentFlat", -0.2, 0.26, 0.44, 0.24, 0, 0.14, "white"),
     C("Ribbon", -0.2, 0.26, 0.06, 0.25, 0, 0.146, "red"), C("FloorPaper", 0, 0, 1.0, 0.8, 0, 0.004, "cream")]
rmodel("festive", "wrapped_presents", c)
c = [C("StandFoot", 0, 0, 0.8, 0.4, 0, 0.05, "dark"), C("StandPole", 0, 0, 0.05, 0.05, 0.05, 1.7, "dark"),
     C("WreathTop", 0, 0.03, 0.8, 0.14, 1.52, 1.7, "dkgreen"), C("WreathBottom", 0, 0.03, 0.8, 0.14, 0.9, 1.08, "dkgreen"), C("WreathSide", -0.31, 0.03, 0.18, 0.14, 1.08, 1.52, "green"),
     C("WreathSide", 0.31, 0.03, 0.18, 0.14, 1.08, 1.52, "green"), C("WreathBow", 0, 0.11, 0.3, 0.03, 0.92, 1.06, "red"), C("BowTail", -0.08, 0.11, 0.07, 0.03, 0.74, 0.92, "red"),
     C("BowTail", 0.08, 0.11, 0.07, 0.03, 0.74, 0.92, "red"), C("Berry", -0.3, 0.105, 0.06, 0.02, 1.56, 1.62, "red"), C("Berry", 0.3, 0.105, 0.06, 0.02, 1.2, 1.26, "red"),
     C("Berry", 0.2, 0.105, 0.06, 0.02, 1.58, 1.64, "gold"), C("Berry", -0.28, 0.105, 0.06, 0.02, 1.14, 1.2, "gold")]
rmodel("festive", "festive_wreath", c)
c = [C("SnowBase", 0, 0, 0.8, 0.8, 0, 0.6, "white"), C("SnowBody", 0, 0, 0.58, 0.58, 0.6, 1.06, "white"), C("SnowHead", 0, 0, 0.4, 0.4, 1.06, 1.4, "white"),
     C("HatBrim", 0, 0, 0.5, 0.5, 1.4, 1.44, "black"), C("HatCrown", 0, 0, 0.3, 0.3, 1.44, 1.6, "black"), C("HatBand", 0, 0, 0.31, 0.31, 1.44, 1.48, "red"),
     C("Scarf", 0, 0, 0.46, 0.46, 1.02, 1.1, "red"), C("ScarfTail", 0.14, 0.292, 0.1, 0.01, 0.8, 1.02, "red"), C("CarrotNose", 0, 0.26, 0.06, 0.14, 1.2, 1.26, "orange"),
     C("CoalEye", -0.08, 0.201, 0.05, 0.006, 1.28, 1.33, "black"), C("CoalEye", 0.08, 0.201, 0.05, 0.006, 1.28, 1.33, "black"), C("CoalButton", 0, 0.291, 0.06, 0.006, 0.9, 0.96, "black"),
     C("CoalButton", 0, 0.291, 0.06, 0.006, 0.72, 0.78, "black"), C("StickArm", -0.34, 0, 0.12, 0.04, 0.88, 0.92, "brown"), C("StickArm", 0.34, 0, 0.12, 0.04, 0.88, 0.92, "brown")]
rmodel("festive", "snowman", c)
c = [C("CaneBase", 0, 0, 0.35, 0.35, 0, 0.08, "white")]
for i in range(8): c.append(C("CaneBand", 0, 0, 0.12, 0.12, 0.08 + i*0.19, 0.08 + (i + 1)*0.19, "red" if i % 2 == 0 else "white"))
c += [C("CaneHook", 0.08, 0, 0.28, 0.12, 1.6, 1.72, "red"), C("CaneHookDrop", 0.165, 0, 0.11, 0.12, 1.44, 1.6, "white"), C("CaneBow", 0, 0.065, 0.2, 0.02, 1.0, 1.1, "green"),
      C("CaneCap", 0, 0, 0.12, 0.12, 1.72, 1.8, "white")]
rmodel("festive", "candy_cane_post", c)
c = [C("MantelJamb", -0.72, 0, 0.36, 0.44, 0, 1.1, "crimson"), C("MantelJamb", 0.72, 0, 0.36, 0.44, 0, 1.1, "crimson"), C("MantelLintel", 0, 0, 1.8, 0.44, 1.1, 1.3, "crimson"),
     C("MantelShelf", 0, 0.03, 1.8, 0.5, 1.3, 1.36, "white"), C("MantelBack", 0, -0.18, 1.08, 0.08, 0, 1.1, "black"), C("Ember", 0, 0, 0.6, 0.2, 0, 0.1, "orange"),
     C("EmberGlow", 0.05, 0.02, 0.3, 0.1, 0.1, 0.3, "amber"), C("Garland", 0, 0.256, 1.7, 0.02, 1.2, 1.28, "green")]
for i, col in enumerate(["red", "green", "white", "red"]):
    x = -0.6 + i*0.4
    c += [C("Stocking", x, 0.262, 0.14, 0.03, 0.82, 1.12, col), C("StockingCuff", x, 0.264, 0.16, 0.034, 1.08, 1.16, "white" if col != "white" else "red"),
          C("StockingToe", x + 0.07, 0.262, 0.14, 0.03, 0.82, 0.92, col)]
c += [C("MantelCandle", -0.7, 0.03, 0.06, 0.06, 1.36, 1.4, "cream"), C("MantelCandle", 0.7, 0.03, 0.06, 0.06, 1.36, 1.4, "cream")]
rmodel("festive", "stocking_mantel", c)
c = [C("Runner", 0, -0.42, 2.2, 0.08, 0, 0.08, "gold"), C("Runner", 0, 0.42, 2.2, 0.08, 0, 0.08, "gold"), C("RunnerCurl", 1.06, -0.42, 0.08, 0.08, 0.08, 0.4, "gold"),
     C("RunnerCurl", 1.06, 0.42, 0.08, 0.08, 0.08, 0.4, "gold"), C("Strut", -0.6, 0, 0.08, 0.92, 0.08, 0.3, "gold"), C("Strut", 0.5, 0, 0.08, 0.92, 0.08, 0.3, "gold"),
     C("SleighFloor", -0.1, 0, 1.8, 1.0, 0.3, 0.42, "red"), C("SleighSide", -0.1, -0.47, 1.8, 0.06, 0.42, 0.8, "red"), C("SleighSide", -0.1, 0.47, 1.8, 0.06, 0.42, 0.8, "red"),
     C("SleighBack", -0.96, 0, 0.08, 1.0, 0.42, 1.1, "red"), C("SleighFront", 0.76, 0, 0.08, 1.0, 0.42, 0.9, "red"), C("SleighSeat", -0.6, 0, 0.5, 0.88, 0.42, 0.62, "green"),
     C("SleighTrim", -0.96, 0, 0.1, 1.0, 1.04, 1.1, "gold"), C("GiftSack", 0.2, 0, 0.6, 0.6, 0.42, 0.9, "brown"), C("SackGift", 0.2, 0, 0.3, 0.3, 0.9, 1.04, "blue"),
     C("TraceEnd", -1.06, 0, 0.08, 0.2, 0.3, 0.38, "gold")]
rmodel("festive", "festive_sleigh", c)
c = [C("NutBase", 0, 0, 0.5, 0.4, 0, 0.1, "black"), C("NutBoot", -0.1, 0, 0.14, 0.2, 0.1, 0.3, "black"), C("NutBoot", 0.1, 0, 0.14, 0.2, 0.1, 0.3, "black"),
     C("NutLeg", -0.1, 0, 0.13, 0.16, 0.3, 0.8, "white"), C("NutLeg", 0.1, 0, 0.13, 0.16, 0.3, 0.8, "white"), C("NutCoat", 0, 0, 0.4, 0.26, 0.8, 1.3, "red"),
     C("NutBelt", 0, 0, 0.41, 0.27, 0.86, 0.94, "black"), C("NutBuckle", 0, 0.136, 0.08, 0.004, 0.86, 0.94, "gold"), C("NutArm", -0.25, 0, 0.1, 0.16, 0.86, 1.28, "red"),
     C("NutArm", 0.25, 0, 0.1, 0.16, 0.86, 1.28, "red"), C("NutEpaulette", -0.25, 0, 0.12, 0.2, 1.28, 1.32, "gold"), C("NutEpaulette", 0.25, 0, 0.12, 0.2, 1.28, 1.32, "gold"),
     C("NutHead", 0, 0, 0.28, 0.26, 1.3, 1.56, "tan"), C("NutBeard", 0, 0.131, 0.2, 0.01, 1.3, 1.4, "white"), C("NutTeeth", 0, 0.132, 0.14, 0.01, 1.4, 1.44, "black"),
     C("NutEye", -0.07, 0.131, 0.05, 0.006, 1.47, 1.52, "black"), C("NutEye", 0.07, 0.131, 0.05, 0.006, 1.47, 1.52, "black"), C("NutHat", 0, 0, 0.3, 0.28, 1.56, 1.76, "black"),
     C("NutHatBand", 0, 0, 0.31, 0.29, 1.56, 1.6, "gold"), C("NutPlume", 0, 0, 0.08, 0.08, 1.76, 1.8, "red")]
rmodel("festive", "nutcracker", c)
c = [C("PostBase", 0, 0, 0.4, 0.4, 0, 0.1, "dark"), C("PostPole", 0, 0, 0.08, 0.08, 0.1, 2.1, "dkgreen"), C("PostCap", 0, 0, 0.16, 0.16, 2.1, 2.2, "gold"),
     C("PostArm", 0, 0, 0.4, 0.04, 1.96, 2.0, "dkgreen"), C("PostArm", 0, 0, 0.04, 0.4, 1.96, 2.0, "dkgreen")]
cols = ["red", "yellow", "lime", "sky", "hotpink", "amber"]
for i in range(9):
    y = 0.3 + i*0.19; s = [(-1, 0), (0, 1), (1, 0), (0, -1)][i % 4]
    c.append(C("Bulb", s[0]*0.07, s[1]*0.07, 0.07, 0.07, y, y + 0.08, cols[i % 6]))
for k, (x, z) in enumerate([(-0.18, 0), (0.18, 0), (0, -0.18), (0, 0.18)]): c.append(C("HangingBulb", x, z, 0.07, 0.07, 1.84, 1.96, cols[k]))
rmodel("festive", "string_light_post", c)

# ---------------- halloween ----------------
c = [C("PumpkinBody", 0, 0, 0.6, 0.6, 0.04, 0.42, "orange"), C("PumpkinFoot", 0, 0, 0.44, 0.44, 0, 0.04, "orange"), C("PumpkinCrown", 0, 0, 0.44, 0.44, 0.42, 0.47, "orange"),
     C("PumpkinStem", 0, 0, 0.08, 0.08, 0.47, 0.55, "dkgreen"), C("PumpkinRib", -0.15, 0.301, 0.02, 0.004, 0.06, 0.4, "brown"), C("PumpkinRib", 0.15, 0.301, 0.02, 0.004, 0.06, 0.4, "brown"),
     C("GlowEye", -0.12, 0.302, 0.1, 0.006, 0.27, 0.35, "yellow"), C("GlowEye", 0.12, 0.302, 0.1, 0.006, 0.27, 0.35, "yellow"), C("GlowNose", 0, 0.302, 0.05, 0.006, 0.21, 0.26, "amber"),
     C("GlowMouth", 0, 0.302, 0.34, 0.006, 0.1, 0.16, "yellow"), C("GlowTooth", -0.08, 0.303, 0.05, 0.006, 0.13, 0.16, "orange"), C("GlowTooth", 0.08, 0.303, 0.05, 0.006, 0.1, 0.13, "orange")]
rmodel("halloween", "jack_o_lantern", c)
c = legs("CauldronLeg", 0.7, 0.7, 0, 0.14, 0.1, "black") + [C("CauldronBelly", 0, 0, 0.9, 0.9, 0.14, 0.62, "black"), C("CauldronNeck", 0, 0, 0.74, 0.74, 0.62, 0.72, "dark"),
     C("CauldronRim", 0, 0, 0.84, 0.84, 0.72, 0.78, "black"), C("Brew", 0, 0, 0.7, 0.7, 0.78, 0.784, "lime"), C("Bubble", -0.14, 0.08, 0.12, 0.12, 0.784, 0.86, "lime"),
     C("Bubble", 0.16, -0.1, 0.08, 0.08, 0.784, 0.84, "green"), C("Ladle", 0.2, 0.16, 0.04, 0.04, 0.78, 0.9, "wood"), C("EmberBed", 0, 0, 0.5, 0.5, 0, 0.05, "orange"),
     C("EmberLog", 0, 0, 0.64, 0.1, 0, 0.08, "brown")]
rmodel("halloween", "witch_cauldron", c)
c = [C("CoffinFoot", 0, 0.7, 0.5, 0.6, 0, 0.5, "brown"), C("CoffinWaist", 0, 0.1, 0.66, 0.6, 0, 0.5, "brown"), C("CoffinShoulder", 0, -0.45, 0.8, 0.5, 0, 0.5, "brown"),
     C("CoffinHead", 0, -0.85, 0.56, 0.3, 0, 0.5, "brown"), C("LidFoot", 0, 0.7, 0.44, 0.56, 0.5, 0.6, "wood"), C("LidWaist", 0, 0.1, 0.6, 0.6, 0.5, 0.6, "wood"),
     C("LidShoulder", 0, -0.45, 0.74, 0.5, 0.5, 0.6, "wood"), C("LidHead", 0, -0.85, 0.5, 0.26, 0.5, 0.6, "wood"), C("LidCross", 0, -0.3, 0.08, 0.7, 0.6, 0.604, "gold"),
     C("LidCross", 0, -0.45, 0.36, 0.08, 0.6, 0.604, "gold"), C("CoffinHandle", -0.335, 0.1, 0.01, 0.3, 0.24, 0.3, "gold"), C("CoffinHandle", 0.335, 0.1, 0.01, 0.3, 0.24, 0.3, "gold")]
rmodel("halloween", "coffin", c)
c = [C("GraveMound", 0, 0, 0.7, 0.25, 0, 0.06, "dkgreen"), C("StoneSlab", 0, 0, 0.56, 0.16, 0.06, 0.8, "grey"), C("StoneShoulder", 0, 0, 0.44, 0.16, 0.8, 0.92, "grey"),
     C("StoneCap", 0, 0, 0.28, 0.16, 0.92, 1.0, "grey"), C("Epitaph", 0, 0.081, 0.3, 0.004, 0.62, 0.66, "dark"), C("Epitaph", 0, 0.081, 0.36, 0.004, 0.52, 0.56, "dark"),
     C("Epitaph", 0, 0.081, 0.24, 0.004, 0.42, 0.46, "dark"), C("StoneCrack", 0.14, 0.081, 0.02, 0.004, 0.1, 0.36, "dark"), C("Moss", -0.2, 0.081, 0.14, 0.006, 0.06, 0.2, "green"),
     C("Moss", 0.1, 0, 0.2, 0.17, 0.995, 1.0, "green")]
rmodel("halloween", "tombstone", c)
c = [C("ScarecrowFoot", 0, 0, 0.4, 0.4, 0, 0.06, "brown"), C("ScarecrowPole", 0, 0, 0.08, 0.08, 0.06, 1.5, "wood"), C("CrossBar", 0, 0, 1.2, 0.07, 1.3, 1.37, "wood"),
     C("Shirt", 0, 0, 0.46, 0.2, 0.9, 1.42, "red"), C("ShirtPatch", 0.1, 0.101, 0.12, 0.004, 1.0, 1.12, "blue"), C("Sleeve", -0.4, 0, 0.34, 0.16, 1.24, 1.42, "red"),
     C("Sleeve", 0.4, 0, 0.34, 0.16, 1.24, 1.42, "red"), C("StrawHand", -0.58, 0, 0.04, 0.12, 1.2, 1.4, "yellow"), C("StrawHand", 0.58, 0, 0.04, 0.12, 1.2, 1.4, "yellow"),
     C("Trousers", 0, 0, 0.4, 0.18, 0.46, 0.9, "navy"), C("StrawHem", 0, 0, 0.36, 0.14, 0.38, 0.46, "yellow"), C("SackHead", 0, 0, 0.3, 0.28, 1.42, 1.72, "tan"),
     C("StitchEye", -0.07, 0.141, 0.05, 0.004, 1.58, 1.63, "black"), C("StitchEye", 0.07, 0.141, 0.05, 0.004, 1.58, 1.63, "black"), C("StitchMouth", 0, 0.141, 0.16, 0.004, 1.49, 1.51, "black"),
     C("HatBrim", 0, 0, 0.5, 0.4, 1.72, 1.76, "brown"), C("HatCrown", 0, 0, 0.26, 0.24, 1.76, 2.0, "brown")]
rmodel("halloween", "scarecrow", c)
c = [C("WebPost", -0.47, 0, 0.06, 0.12, 0, 1.6, "dark"), C("WebPost", 0.47, 0, 0.06, 0.12, 0, 1.6, "dark"), C("WebBeam", 0, 0, 1.0, 0.12, 1.54, 1.6, "dark")]
for i in range(6):
    y = 1.5 - i*0.18; w = 0.88 - i*0.14
    c.append(C("WebStrand", 0, 0, w, 0.01, y, y + 0.02, "white"))
for x in (-0.3, -0.1, 0.1, 0.3): c.append(C("WebSpoke", x, 0, 0.02, 0.01, 1.54 - (0.8 if abs(x) < 0.2 else 0.45), 1.54, "white"))
c += [C("Spider", 0.08, 0.02, 0.1, 0.03, 0.98, 1.06, "black"), C("SpiderLegs", 0.08, 0.02, 0.2, 0.02, 1.0, 1.02, "black"), C("SpiderThread", 0.08, 0.01, 0.01, 0.01, 1.06, 1.3, "white"),
      C("WebFloor", 0, 0, 1.0, 0.12, 0, 0.01, "grey")]
rmodel("halloween", "cobweb_corner", c)
c = [C("BoneStandFoot", 0, 0, 0.5, 0.3, 0, 0.05, "dark"), C("BoneStandPole", 0, -0.1, 0.04, 0.04, 0.05, 1.8, "dark"), C("Shin", -0.09, 0, 0.05, 0.05, 0.05, 0.48, "white"),
     C("Shin", 0.09, 0, 0.05, 0.05, 0.05, 0.48, "white"), C("Thigh", -0.09, 0, 0.06, 0.06, 0.48, 0.9, "cream"), C("Thigh", 0.09, 0, 0.06, 0.06, 0.48, 0.9, "cream"),
     C("Pelvis", 0, 0, 0.3, 0.12, 0.9, 1.0, "white"), C("Spine", 0, 0, 0.05, 0.06, 1.0, 1.46, "cream")]
for i in range(4): c.append(C("Rib", 0, 0.01, 0.3 - i*0.02, 0.14, 1.14 + i*0.08, 1.18 + i*0.08, "white"))
c += [C("Collarbone", 0, 0, 0.4, 0.05, 1.46, 1.5, "white"), C("ArmBone", -0.21, 0, 0.04, 0.04, 0.96, 1.46, "cream"), C("ArmBone", 0.21, 0, 0.04, 0.04, 0.96, 1.46, "cream"),
      C("Skull", 0, 0.01, 0.2, 0.22, 1.54, 1.76, "white"), C("Jaw", 0, 0.02, 0.14, 0.16, 1.5, 1.54, "cream"), C("EyeSocket", -0.05, 0.121, 0.05, 0.004, 1.63, 1.69, "black"),
      C("EyeSocket", 0.05, 0.121, 0.05, 0.004, 1.63, 1.69, "black"), C("NoseSocket", 0, 0.121, 0.03, 0.004, 1.58, 1.62, "black")]
rmodel("halloween", "skeleton_prop", c)
c = [C("RackFoot", -0.4, 0, 0.1, 0.35, 0, 0.06, "brown"), C("RackFoot", 0.4, 0, 0.1, 0.35, 0, 0.06, "brown"), C("RackPost", -0.4, -0.1, 0.06, 0.06, 0.06, 1.3, "brown"),
     C("RackPost", 0.4, -0.1, 0.06, 0.06, 0.06, 1.3, "brown"), C("RackRail", 0, -0.1, 0.9, 0.06, 1.2, 1.26, "wood"), C("RackRail", 0, -0.1, 0.9, 0.06, 0.3, 0.36, "wood")]
for i, x in enumerate([-0.22, 0.0, 0.22]):
    c += [C("BroomHandle", x, 0, 0.04, 0.04, 0.4, 1.6, "wood"), C("BroomBinding", x, 0, 0.1, 0.1, 0.36, 0.44, "black"), C("BroomBristles", x, 0, 0.16, 0.14, 0.04, 0.36, "amber" if i != 1 else "tan")]
c += [C("WitchHat", 0.33, 0.02, 0.24, 0.24, 1.3, 1.34, "purple"), C("WitchHatCone", 0.33, 0.02, 0.12, 0.12, 1.34, 1.56, "purple")]
rmodel("halloween", "witch_broom_rack", c)
c = [C("WaxPool", 0, 0, 0.5, 0.5, 0, 0.02, "cream")]
for (x, z, h, w) in [(-0.1, -0.08, 0.34, 0.12), (0.1, -0.1, 0.22, 0.1), (0.0, 0.1, 0.42, 0.11), (-0.16, 0.12, 0.16, 0.09), (0.17, 0.08, 0.28, 0.09)]:
    c += [C("Candle", x, z, w, w, 0.02, h, "cream" if h > 0.25 else "white"), C("Wick", x, z, 0.015, 0.015, h, h + 0.03, "black"), C("CandleFlame", x, z, 0.04, 0.04, h + 0.03, h + 0.08, "amber")]
rmodel("halloween", "candle_cluster", c)

# ---------------- harvest ----------------
c = [C("HayBlock", 0, 0, 1.2, 0.7, 0, 0.7, "amber"), C("HayTwine", -0.3, 0, 0.04, 0.71, 0, 0.706, "brown"), C("HayTwine", 0.3, 0, 0.04, 0.71, 0, 0.706, "brown"),
     C("HayEnd", -0.602, 0, 0.004, 0.6, 0.06, 0.64, "yellow"), C("HayEnd", 0.602, 0, 0.004, 0.6, 0.06, 0.64, "yellow"), C("HayTuft", 0.1, 0.1, 0.3, 0.2, 0.7, 0.704, "yellow")]
rmodel("harvest", "hay_bale", c)
def table(w, d, h, col="wood", leg="brown"):
    return legs("TableLeg", w - 0.1, d - 0.1, 0, h - 0.06, 0.09, leg) + [C("TableTop", 0, 0, w, d, h - 0.06, h, col)]
c = table(1.4, 0.8, 0.78) + [C("TableRunner", 0, 0, 0.5, 0.8, 0.78, 0.786, "crimson"), C("HornMouth", -0.2, 0, 0.36, 0.34, 0.786, 1.1, "tan"), C("HornMid", 0.08, 0, 0.26, 0.26, 0.786, 1.02, "wood"),
     C("HornTail", 0.3, 0, 0.2, 0.16, 0.786, 0.94, "brown"), C("HornTip", 0.45, 0, 0.1, 0.08, 0.84, 0.98, "brown"), C("Gourd", -0.5, 0.1, 0.18, 0.18, 0.786, 0.94, "orange"),
     C("Apple", -0.45, -0.14, 0.1, 0.1, 0.786, 0.88, "red"), C("Grapes", -0.42, 0.28, 0.14, 0.12, 0.786, 0.86, "purple"), C("Corn", -0.56, -0.26, 0.2, 0.07, 0.786, 0.85, "yellow"),
     C("Pear", -0.3, -0.3, 0.08, 0.08, 0.786, 0.9, "lime")]
rmodel("harvest", "cornucopia_table", c)
c = [C("PumpkinBig", -0.2, -0.1, 0.56, 0.56, 0, 0.46, "orange"), C("PumpkinStem", -0.2, -0.1, 0.08, 0.08, 0.46, 0.56, "dkgreen"), C("PumpkinMid", 0.3, 0.1, 0.44, 0.44, 0, 0.36, "amber"),
     C("PumpkinStem", 0.3, 0.1, 0.07, 0.07, 0.36, 0.44, "dkgreen"), C("PumpkinSmall", -0.1, 0.34, 0.3, 0.3, 0, 0.24, "orange"), C("PumpkinStem", -0.1, 0.34, 0.05, 0.05, 0.24, 0.3, "brown"),
     C("PumpkinTop", 0.02, -0.02, 0.36, 0.36, 0.46, 0.7, "orange"), C("PumpkinStem", 0.02, -0.02, 0.06, 0.06, 0.46, 0.5, "dkgreen"), C("WhiteGourd", 0.36, -0.3, 0.24, 0.24, 0, 0.2, "cream"),
     C("StrawBed", 0, 0, 1.2, 1.0, 0, 0.01, "yellow")]
rmodel("harvest", "pumpkin_pile", c)
c = [C("BundleFoot", 0, 0, 0.6, 0.6, 0, 0.02, "yellow"), C("BundleCore", 0, 0, 0.3, 0.3, 0.02, 1.5, "tan"), C("BundleTie", 0, 0, 0.34, 0.34, 0.8, 0.9, "crimson")]
for k, (x, z) in enumerate([(-0.18, -0.18), (0.18, -0.16), (-0.16, 0.18), (0.18, 0.18), (0, 0)]):
    h = 2.0 if k == 4 else 1.7 + 0.06*k
    c += [C("Stalk", x, z, 0.06, 0.06, 0.02, h, "amber" if k % 2 else "yellow"), C("Tassel", x, z, 0.14, 0.14, h - 0.22, h - 0.1, "tan")]
c += [C("CornEar", 0.17, 0.02, 0.07, 0.07, 1.0, 1.24, "yellow"), C("CornEar", -0.17, -0.02, 0.07, 0.07, 1.12, 1.34, "gold"), C("Leaf", 0.21, -0.1, 0.02, 0.2, 0.5, 1.1, "green"),
      C("Leaf", -0.21, 0.1, 0.02, 0.2, 0.4, 1.0, "green")]
rmodel("harvest", "corn_stalk_bundle", c)
c = table(2.4, 1.1, 0.8) + [C("FeastCloth", 0, 0, 2.2, 0.9, 0.8, 0.806, "cream"), C("FeastRunner", 0, 0, 2.3, 0.3, 0.806, 0.81, "orange"), C("RoastPlatter", 0, 0, 0.6, 0.44, 0.81, 0.84, "metal"),
     C("RoastBird", 0, 0, 0.4, 0.3, 0.84, 1.04, "brown"), C("RoastLeg", -0.22, 0.1, 0.08, 0.08, 0.84, 0.98, "tan"), C("RoastLeg", 0.22, 0.1, 0.08, 0.08, 0.84, 0.98, "tan"),
     C("PieDish", -0.75, 0, 0.3, 0.3, 0.81, 0.87, "orange"), C("BreadLoaf", 0.75, 0.1, 0.34, 0.16, 0.81, 0.93, "tan"), C("GravyBoat", 0.5, -0.25, 0.16, 0.1, 0.81, 0.9, "white"),
     C("FeastCandle", -0.42, 0, 0.05, 0.05, 0.81, 1.1, "cream"), C("FeastCandle", 0.42, 0, 0.05, 0.05, 0.81, 1.1, "cream")]
for x in (-0.9, -0.3, 0.3, 0.9):
    for z in (-0.38, 0.38): c.append(C("Plate", x, z, 0.24, 0.2, 0.806, 0.82, "white"))
rmodel("harvest", "harvest_feast_table", c)
c = [C("PieStandFoot", 0, 0, 0.8, 0.5, 0, 0.06, "brown"), C("PieStandPost", -0.36, 0, 0.06, 0.4, 0.06, 1.3, "wood"), C("PieStandPost", 0.36, 0, 0.06, 0.4, 0.06, 1.3, "wood"),
     C("PieStandSign", 0, 0, 0.8, 0.06, 1.14, 1.3, "crimson"), C("SignLetters", 0, 0.031, 0.5, 0.004, 1.19, 1.25, "cream")]
for i, y in enumerate([0.36, 0.7, 1.0]):
    c.append(C("PieShelf", 0, 0, 0.72, 0.46, y, y + 0.04, "wood"))
    if i < 2:
        for k, x in enumerate([-0.18, 0.18]):
            c += [C("PieCrust", x, 0, 0.28, 0.28, y + 0.04, y + 0.1, "tan"), C("PieFilling", x, 0, 0.2, 0.2, y + 0.1, y + 0.114, ["orange", "crimson", "purple", "amber"][i*2 + k])]
    else: c += [C("PieCrust", 0, 0, 0.3, 0.3, y + 0.04, y + 0.1, "tan"), C("PieLattice", 0, 0, 0.22, 0.22, y + 0.1, y + 0.114, "wood")]
rmodel("harvest", "pie_stand", c)
c = [C("PlinthBase", 0, 0, 0.7, 0.7, 0, 0.08, "brown"), C("PlinthColumn", 0, 0, 0.5, 0.5, 0.08, 0.6, "wood"), C("PlinthTop", 0, 0, 0.66, 0.66, 0.6, 0.66, "brown"),
     C("TurkeyBody", 0, 0.02, 0.3, 0.34, 0.72, 0.96, "brown"), C("TurkeyFoot", -0.07, 0.04, 0.04, 0.1, 0.66, 0.72, "amber"), C("TurkeyFoot", 0.07, 0.04, 0.04, 0.1, 0.66, 0.72, "amber"),
     C("TurkeyNeck", 0, 0.16, 0.1, 0.1, 0.96, 1.04, "brown"), C("TurkeyHead", 0, 0.18, 0.12, 0.12, 1.0, 1.1, "red"), C("TurkeyBeak", 0, 0.26, 0.04, 0.05, 1.03, 1.07, "amber"),
     C("TurkeyWattle", 0, 0.245, 0.03, 0.02, 0.96, 1.03, "crimson"), C("TurkeyWing", -0.17, 0.02, 0.04, 0.24, 0.76, 0.92, "tan"), C("TurkeyWing", 0.17, 0.02, 0.04, 0.24, 0.76, 0.92, "tan")]
for i, (x, h, col) in enumerate([(-0.24, 0.98, "orange"), (-0.12, 1.06, "crimson"), (0, 1.1, "gold"), (0.12, 1.06, "crimson"), (0.24, 0.98, "orange")]):
    c.append(C("TailFeather", x, -0.17, 0.1, 0.04, 0.76, h, col))
rmodel("harvest", "turkey_centrepiece", c)
c = [C("LeafBed", 0, 0, 1.2, 1.1, 0, 0.02, "brown"), C("LeafHeap", 0, 0, 0.9, 0.8, 0.02, 0.14, "orange"), C("LeafHeap", -0.05, 0.05, 0.6, 0.5, 0.14, 0.24, "amber"),
     C("LeafHeap", 0.05, 0, 0.3, 0.26, 0.24, 0.3, "crimson"), C("LooseLeaf", -0.48, 0.4, 0.14, 0.12, 0.02, 0.03, "red"), C("LooseLeaf", 0.46, -0.42, 0.16, 0.12, 0.02, 0.03, "yellow"),
     C("LooseLeaf", 0.42, 0.36, 0.12, 0.14, 0.02, 0.03, "orange"), C("LooseLeaf", -0.44, -0.38, 0.14, 0.14, 0.02, 0.03, "gold")]
rmodel("harvest", "leaf_pile", c)
c = [C("CartBed", 0, 0, 1.4, 0.8, 0.5, 0.58, "wood"), C("CartSide", 0, -0.37, 1.4, 0.06, 0.58, 0.9, "brown"), C("CartSide", 0, 0.37, 1.4, 0.06, 0.58, 0.9, "brown"),
     C("CartEnd", -0.67, 0, 0.06, 0.8, 0.58, 0.9, "brown"), C("CartEnd", 0.67, 0, 0.06, 0.8, 0.58, 0.9, "brown"), C("Axle", 0, 0, 0.08, 1.0, 0.36, 0.44, "dark"),
     C("AxleBlock", 0, 0, 0.2, 0.7, 0.44, 0.5, "brown")]
for z in (-0.47, 0.47):
    c += [C("WheelRim", 0, z, 0.8, 0.06, 0, 0.08, "dark"), C("WheelRim", 0, z, 0.8, 0.06, 0.72, 0.8, "dark"), C("WheelRim", -0.36, z, 0.08, 0.06, 0.08, 0.72, "dark"),
          C("WheelRim", 0.36, z, 0.08, 0.06, 0.08, 0.72, "dark"), C("WheelSpoke", 0, z, 0.64, 0.04, 0.37, 0.43, "wood"), C("WheelSpoke", 0, z, 0.06, 0.04, 0.08, 0.72, "wood")]
c += [C("Shaft", 0.8, -0.3, 0.2, 0.06, 0.5, 0.56, "wood"), C("Shaft", 0.8, 0.3, 0.2, 0.06, 0.5, 0.56, "wood"), C("Shaft", -0.8, -0.3, 0.2, 0.06, 0.5, 0.56, "wood"),
      C("Shaft", -0.8, 0.3, 0.2, 0.06, 0.5, 0.56, "wood"), C("CartPumpkin", -0.3, 0, 0.36, 0.36, 0.58, 0.9, "orange"), C("CartPumpkin", 0.2, 0.1, 0.3, 0.3, 0.58, 0.84, "amber"),
      C("CartHay", 0.2, -0.1, 0.6, 0.4, 0.58, 0.7, "yellow"), C("CartSquash", 0.42, 0.14, 0.16, 0.2, 0.84, 1.1, "cream")]
rmodel("harvest", "wooden_cart", c)

print("REGION_PASS_SIZES")
for folder, name, size in REGION_PASS: print("  props/%s/%s.bbmodel %s" % (folder, name, size))
print(len(REGION_PASS), "regions pass models")
