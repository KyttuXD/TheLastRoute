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
Only reads the original project.
"""
import json
import os
import struct
import sys

GAMEPLAY = ["idle", "walk", "chase", "sprint", "crouch", "charge", "flight", "jumpscare", "climb", "stun", "stun_recover"]
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


def extract(glb_path, bb_path, stride):
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
        wanted = name in GAMEPLAY or name.startswith("scythe_") or ("--emotes" in sys.argv and name.startswith("emote_"))
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


def main():
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
