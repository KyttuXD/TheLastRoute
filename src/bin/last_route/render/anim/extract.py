"""Extracts the blocky hunters' animation clips from the original's GLBs into assets/hunters/anim/<id>.json.

The original (Godot) plays these clips through an AnimationPlayer: tools/blocky_hunters/bb_gltf.py writes them into
assets/hunters_blocky/<id>/<id>.glb as node TRS tracks (LINEAR), one node per bbmodel outliner group. The port draws the
same hunters from the .bbmodel, so this keeps only what the bbmodel lacks:

  unit   bbmodel units per glTF metre (bb_gltf.py: pivots x64 in the bbmodel, x S = 64/65 in the glb -> 65)
  bones  [name, parent index (-1 = model root), rest translation (bbmodel units), rest rotation (x, y, z, w)]
         -- the glb's rest pose, which carries the arm/thigh splay that the bbmodel groups (rotation 0) do not
  clips  name -> {"len": seconds (Godot: max key time), "loop": bool (the "-loop" suffix, stripped as Godot's importer does),
                  "tracks": [[bone index, "t"|"r", [times], [values flat]]]}
         -- the SOURCE keys; render/anim bakes them on load exactly like Godot's glTF importer (animation/fps = 30).
            Tracks that only hold the node's rest value are dropped here (animation/remove_immutable_tracks = true).
  stride clip -> {"ref", "reverse"} from assets/hunters_blocky/stride_table.json (HunterStride, measured per model)

Usage: python -I extract.py <original KyttusLabApp dir> <port app dir> [--emotes]
       python -I extract.py --player <original KyttusLabApp dir> <port app dir>   (runner mannequin -> assets/player/anim/)
       python -I extract.py --check <original KyttusLabApp dir> <port app dir>    (self-check of the player data)
Only reads the original project.
"""
import json
import os
import struct
import sys

GAMEPLAY = ["idle", "walk", "chase", "sprint", "crouch", "charge", "flight", "jumpscare", "climb", "stun", "stun_recover"]
PLAYER_CLIPS = ["idle", "walk", "chase", "sprint", "crouch", "stun", "stun_recover", "tpose", "sit", "lay", "jump", "fall", "climb"]
PLAYER_RIGS = [("player_mannequin", "player_mannequin"), ("player_mannequin_slim", "player_mannequin_slim")]
CMP_EPSILON = 0.00001


def read_glb(path):
    b = open(path, "rb").read()
    jlen = struct.unpack("<I", b[12:16])[0]
    j = json.loads(b[20:20 + jlen])
    bin_off = 20 + jlen + 8

    def acc(i):
        a = j["accessors"][i]
        v = j["bufferViews"][a["bufferView"]]
        n = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}[a["type"]]
        off = bin_off + v.get("byteOffset", 0) + a.get("byteOffset", 0)
        stride = v.get("byteStride", 4 * n)
        return [list(struct.unpack_from("<%df" % n, b, off + k * stride)) for k in range(a["count"])]

    return j, acc


def approx(a, b):
    # Math::is_equal_approx (Godot): tolerance CMP_EPSILON * |a|, at least CMP_EPSILON
    for x, y in zip(a, b):
        if x == y:
            continue
        tol = max(CMP_EPSILON * abs(x), CMP_EPSILON)
        if abs(x - y) >= tol:
            return False
    return True


def norm(q):
    l = sum(c * c for c in q) ** 0.5
    return [c / l for c in q] if l > 0 else [0.0, 0.0, 0.0, 1.0]


def outliner_origins(bb):
    out = {}

    def walk(nodes, parent):
        for n in nodes:
            if isinstance(n, dict):
                out[n["name"]] = (n["origin"], parent)
                walk(n.get("children", []), n["name"])

    walk(bb["outliner"], None)
    return out


def r5(x):
    return round(x, 5)


def extract(glb_path, bb_path, stride, only=None):
    j, acc = read_glb(glb_path)
    nodes = j["nodes"]
    parent = {}
    for i, n in enumerate(nodes):
        for c in n.get("children", []):
            parent[c] = i
    root = j["scenes"][j.get("scene", 0)]["nodes"][0]
    # depth-first order from the model root: parents always before children
    order = []

    def walk(i):
        for c in nodes[i].get("children", []):
            order.append(c)
            walk(c)

    walk(root)
    index = {n: k for k, n in enumerate(order)}
    groups = outliner_origins(json.load(open(bb_path, encoding="utf-8")))
    # bbmodel units per glb metre, checked on every bone that has an outliner group
    unit = None
    worst = 0.0
    for gi in order:
        name = nodes[gi].get("name")
        t = nodes[gi].get("translation", [0, 0, 0])
        if name in groups and abs(t[1]) > 1e-4:
            origin, par = groups[name]
            po = groups[par][0] if par else [0, 0, 0]
            ratio = (origin[1] - po[1]) / t[1]
            if unit is None:
                unit = round(ratio, 4)
            worst = max(worst, abs(ratio - unit))
    assert unit is not None and worst < 1e-2, (glb_path, unit, worst)
    bones = []
    for gi in order:
        n = nodes[gi]
        t = n.get("translation", [0.0, 0.0, 0.0])
        r = norm(n.get("rotation", [0.0, 0.0, 0.0, 1.0]))
        p = parent.get(gi)
        bones.append([n.get("name", "node%d" % gi), index.get(p, -1), [r5(v * unit) for v in t], [r5(v) for v in r]])
    clips = {}
    for an in j.get("animations", []):
        name = an["name"]
        loop = False
        for suffix in ("-loop", "_loop"):
            if name.endswith(suffix):
                name, loop = name[: -len(suffix)], True
        wanted = (name in only) if only is not None else name in GAMEPLAY or name.startswith("scythe_") or ("--emotes" in sys.argv and name.startswith("emote_"))
        if not wanted:
            continue
        end = 0.0
        tracks = []
        for ch in an["channels"]:
            s = an["samplers"][ch["sampler"]]
            assert s.get("interpolation", "LINEAR") == "LINEAR", s
            times = [v[0] for v in acc(s["input"])]
            vals = acc(s["output"])
            end = max(end, max(times))
            gi = ch["target"]["node"]
            path = ch["target"]["path"]
            if path not in ("translation", "rotation"):
                continue
            n = nodes[gi]
            if path == "rotation":
                vals = [norm(v) for v in vals]
                base = norm(n.get("rotation", [0.0, 0.0, 0.0, 1.0]))
            else:
                base = n.get("translation", [0.0, 0.0, 0.0])
            if all(approx(v, base) for v in vals):
                continue   # remove_immutable_tracks: the node keeps its rest value
            if path == "translation":
                vals = [[c * unit for c in v] for v in vals]
            tracks.append([index[gi], "t" if path == "translation" else "r", [r5(t) for t in times],
                           [r5(c) for v in vals for c in v]])
        clips[name] = {"len": r5(end), "loop": loop, "tracks": tracks}
    out = {"unit": unit, "bones": bones, "clips": clips}
    if stride:
        out["stride"] = {k: {"ref": v["ref"], "reverse": bool(v.get("reverse", False))} for k, v in stride.items()
                         if not k.startswith("nw/")}
    return out


def player_paths(orig, app, rig):
    src = os.path.join(orig, "assets", "hunters_blocky", rig)
    return os.path.join(src, rig + ".glb"), os.path.join(src, rig + ".bbmodel"), src, os.path.join(app, "assets", "player")


def player_main(orig, app):
    import shutil
    table = json.load(open(os.path.join(orig, "assets", "hunters_blocky", "stride_table.json"), encoding="utf-8"))
    for rig, _ in PLAYER_RIGS:
        glb, bb, src, dst = player_paths(orig, app, rig)
        os.makedirs(os.path.join(dst, "anim"), exist_ok=True)
        if rig.endswith("_slim"):
            for f in (rig + ".bbmodel", rig + "_atlas.png"):
                shutil.copyfile(os.path.join(src, f), os.path.join(dst, f))
        data = extract(glb, bb, table.get(rig), only=PLAYER_CLIPS)
        path = os.path.join(dst, "anim", rig + ".json")
        with open(path, "w", encoding="utf-8", newline="\n") as fh:
            json.dump(data, fh, separators=(",", ":"))
        print(rig, "unit", data["unit"], "bones", len(data["bones"]), "clips", len(data["clips"]), os.path.getsize(path))
    # the slim rig must be driven by the regular rig's bones: report any group-name difference
    names = []
    for rig, _ in PLAYER_RIGS:
        names.append(set(outliner_origins(json.load(open(player_paths(orig, app, rig)[1], encoding="utf-8")))))
    print("slim vs regular group names:", "identical" if names[0] == names[1] else
          "DIFFER only-regular=%s only-slim=%s" % (sorted(names[0] - names[1]), sorted(names[1] - names[0])))


def player_check(orig, app):
    """Asserts: 13 clips, every bone is a bbmodel group, clip lengths equal the GLB's own max key time."""
    for rig, _ in PLAYER_RIGS:
        glb, bb, src, dst = player_paths(orig, app, rig)
        data = json.load(open(os.path.join(dst, "anim", rig + ".json"), encoding="utf-8"))
        assert sorted(data["clips"]) == sorted(PLAYER_CLIPS), (rig, sorted(data["clips"]))
        assert len(data["clips"]) == 13, rig
        groups = set(outliner_origins(json.load(open(os.path.join(dst, rig + ".bbmodel") if rig.endswith("_slim")
                                                    else os.path.join(dst, rig + ".bbmodel"), encoding="utf-8"))))
        for b in data["bones"]:
            # eye attachments and hand props are glb-only mount nodes (no bbmodel group); every other bone is a group
            assert b[0] in groups or b[0].endswith(("Attachment", "Prop")), (rig, "bone not in bbmodel", b[0])
        j, acc = read_glb(glb)
        want = {}
        for an in j["animations"]:
            n = an["name"]
            for suffix in ("-loop", "_loop"):
                if n.endswith(suffix):
                    n = n[: -len(suffix)]
            want[n] = max(max(v[0] for v in acc(s["input"])) for s in an["samplers"])
        for name, c in data["clips"].items():
            assert abs(c["len"] - want[name]) < 1e-4, (rig, name, c["len"], want[name])
        for name, ln in (("idle", 2.4), ("walk", 1.0), ("chase", 0.5), ("sprint", 0.67)):
            assert abs(data["clips"][name]["len"] - ln) < 0.01, (rig, name, data["clips"][name]["len"])
        assert data["clips"]["idle"]["loop"] and data["clips"]["walk"]["loop"], rig
        print(rig, "ok:", len(data["clips"]), "clips,", len(data["bones"]), "bones")


def main():
    if sys.argv[1] == "--player":
        return player_main(sys.argv[2], sys.argv[3])
    if sys.argv[1] == "--check":
        return player_check(sys.argv[2], sys.argv[3])
    orig, app = sys.argv[1], sys.argv[2]
    src = os.path.join(orig, "assets", "hunters_blocky")
    table = json.load(open(os.path.join(src, "stride_table.json"), encoding="utf-8"))
    hunters = os.path.join(app, "assets", "hunters")
    dst = os.path.join(hunters, "anim")
    os.makedirs(dst, exist_ok=True)
    for f in sorted(os.listdir(hunters)):
        if not f.endswith(".bbmodel"):
            continue
        hid = f[: -len(".bbmodel")]
        glb = os.path.join(src, hid, hid + ".glb")
        if not os.path.exists(glb):
            print("no glb for", hid)
            continue
        data = extract(glb, os.path.join(hunters, f), table.get(hid))
        path = os.path.join(dst, hid + ".json")
        with open(path, "w", encoding="utf-8") as fh:
            json.dump(data, fh, separators=(",", ":"))
        print(hid, "unit", data["unit"], "bones", len(data["bones"]), "clips", len(data["clips"]), os.path.getsize(path))


if __name__ == "__main__":
    main()
