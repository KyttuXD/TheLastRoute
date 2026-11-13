# Writes the 20 arcade and carnival models of the props pass (docs/PROPS_PASS.md): cubes only, one shared palette
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
