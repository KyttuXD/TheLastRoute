"""Builds a creature rig (assets/hunters/anim/<id>.json) from a Blockbench model that carries its own animations.

The humanoid hunters take their rig from the original's GLBs (extract.py). The secret wolf spiders have no glb in this port:
the model is the source .bbmodel (tools/city/ of the original) and the clips are its own Blockbench animations, written out
here in the same `lr.rig` json that render/anim/clip.rs reads:

  unit    1 / metres (kept for the schema; a creature rig sets `metres` too, which wins)
  metres  metres per bbmodel unit (0.035 for the spiders: the head about 2 m up, the legs 3.4 m across)
  bones   [name, parent index (-1 = root), rest translation (bbmodel units, relative to the parent), rest rotation xyzw]
          one bone per outliner group; the root group sits at the model origin (feet centre), every other group at
          its pivot minus its parent's pivot, rest rotation 0 (the spider bbmodels have none)
  clips   name -> {"len", "loop", "tracks": [[bone, "t"|"r", [times], [values]]]}, translations as rest + keyframe, rotations
          as the keyframe's Blockbench ZYX euler (degrees) turned into a quaternion, sign-continuous so a slerp never goes the
          long way round; a looping clip is closed on its first key and every track starts at 0 (the original's converter,
          tools/city/bbmodel_creature.py, does the same, so the pose matches what the original draws)
  stride  clip -> {"ref": ground speed in metres per second at the drawn size, "reverse": false}, measured from the
          planted feet of the clip (the feet's backward travel while they are on the ground)

The black wolf spider's model has no animations: its clips are the white spider's, retargeted bone by bone by the original's
tools/city/spider_clip_map.py into black_wolf_spider.clips.json (`--clips`).

Usage: python3 -I extract_creature.py <model.bbmodel> <out.json> [--clips <clips.json>] [--metres 0.035]
"""
import json
import math
import sys


def q_mul(a, b):
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return (aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx,
            aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz)


def q_axis(axis, deg):
    h = math.radians(deg) / 2
    v = [0.0, 0.0, 0.0]
    v[axis] = math.sin(h)
    return (v[0], v[1], v[2], math.cos(h))


def q_euler_zyx(rx, ry, rz):
    """Blockbench's rotation order: X first, then Y, then Z."""
    return q_mul(q_axis(2, rz), q_mul(q_axis(1, ry), q_axis(0, rx)))


def rotate(q, v):
    x, y, z, w = q
    tx, ty, tz = 2 * (y * v[2] - z * v[1]), 2 * (z * v[0] - x * v[2]), 2 * (x * v[1] - y * v[0])
    return (v[0] + w * tx + (y * tz - z * ty), v[1] + w * ty + (z * tx - x * tz), v[2] + w * tz + (x * ty - y * tx))


def r5(x):
    return round(x, 5)


def read_model(path):
    bb = json.load(open(path))
    bones, uuids = [], {}

    def walk(node, parent, parent_origin):
        origin = node.get("origin", [0, 0, 0])
        index = len(bones)
        rest = [origin[i] - parent_origin[i] for i in range(3)]
        bones.append({"name": node["name"], "parent": -1 if parent is None else parent, "rest": rest, "origin": origin})
        uuids[node["uuid"]] = index
        for child in node.get("children", []):
            if isinstance(child, dict):
                walk(child, index, origin)

    roots = [g for g in bb["outliner"] if isinstance(g, dict)]
    # a top-level group (the root, the hidden hitbox) sits at the model origin: its rest translation is zero
    for g in roots:
        walk(g, None, g.get("origin", [0, 0, 0]))
    return bb, bones, uuids


def own_clips(bb, uuids):
    for a in bb.get("animations", []):
        tracks = {}
        for auuid, an in a["animators"].items():
            if auuid not in uuids:
                continue
            tr = {"rotation": {}, "position": {}}
            for k in an.get("keyframes", []):
                if k["channel"] in tr and k.get("data_points"):
                    dp = k["data_points"][0]
                    tr[k["channel"]][round(k["time"], 6)] = (float(dp.get("x", 0) or 0), float(dp.get("y", 0) or 0), float(dp.get("z", 0) or 0))
            tracks[uuids[auuid]] = tr
        yield {"name": a["name"], "length": a["length"], "loop": a.get("loop") == "loop", "tracks": tracks}


def retargeted_clips(path, names):
    for cname, c in json.load(open(path))["clips"].items():
        tracks = {}
        for bname, chans in c["bones"].items():
            if bname not in names:
                raise SystemExit("clips json drives a bone the model does not have: %s" % bname)
            tr = {"rotation": {}, "position": {}}
            for chan in ("rotation", "position"):
                for key in chans.get(chan, []):
                    tr[chan][round(key[0], 6)] = (key[1], key[2], key[3])
            tracks[names[bname]] = tr
        yield {"name": cname, "length": c["length"], "loop": bool(c["loop"]), "tracks": tracks}


def build_clip(src, bones):
    tracks, longest = [], 0.0
    for bone, by_chan in sorted(src["tracks"].items()):
        for chan, keys in by_chan.items():
            if not keys:
                continue
            times = sorted(keys)
            if src["loop"] and times[-1] < src["length"] - 1e-6:
                keys[round(src["length"], 6)] = keys[times[0]]
                times = sorted(keys)
            if times[0] > 1e-6:
                keys[0.0] = keys[times[0]]
                times = sorted(keys)
            vals = [keys[t] for t in times]
            if chan == "rotation":
                out, prev = [], None
                for v in vals:
                    q = q_euler_zyx(*v)
                    if prev is not None and sum(q[i] * prev[i] for i in range(4)) < 0:
                        q = tuple(-c for c in q)
                    out.append(q)
                    prev = q
                flat = [r5(c) for q in out for c in q]
                kind = "r"
            else:
                rest = bones[bone]["rest"]
                flat = [r5(rest[i] + v[i]) for v in vals for i in range(3)]
                kind = "t"
            tracks.append([bone, kind, [r5(t) for t in times], flat])
            longest = max(longest, times[-1])
    return {"len": r5(longest), "loop": src["loop"], "tracks": tracks}


def sample(track, t):
    times, flat, kind = track[2], track[3], track[1]
    n = 4 if kind == "r" else 3
    if t <= times[0]:
        return flat[:n]
    if t >= times[-1]:
        return flat[-n:]
    i = max(k for k in range(len(times)) if times[k] <= t)
    c = (t - times[i]) / (times[i + 1] - times[i])
    a, b = flat[i * n:(i + 1) * n], flat[(i + 1) * n:(i + 2) * n]
    out = [a[k] + (b[k] - a[k]) * c for k in range(n)]
    if kind == "r":
        l = math.sqrt(sum(x * x for x in out)) or 1.0
        out = [x / l for x in out]
    return out


def foot_positions(bones, clip, t):
    local = [(list(b["rest"]), (0.0, 0.0, 0.0, 1.0)) for b in bones]
    for tr in clip["tracks"]:
        v = sample(tr, t)
        pos, rot = local[tr[0]]
        local[tr[0]] = (v, rot) if tr[1] == "t" else (pos, tuple(v))
    world = []
    for k, b in enumerate(bones):
        pos, rot = local[k]
        if b["parent"] < 0:
            world.append((tuple(pos), rot))
        else:
            pp, pr = world[b["parent"]]
            r = rotate(pr, pos)
            world.append(((pp[0] + r[0], pp[1] + r[1], pp[2] + r[2]), q_mul(pr, rot)))
    return {b["name"]: world[k][0] for k, b in enumerate(bones) if "foot" in b["name"].lower()}


def ground_speed(bones, clip, metres):
    """The planted feet's backward travel (+Z: the head faces -Z), in metres per second."""
    length, steps = clip["len"], 120
    frames = [foot_positions(bones, clip, length * s / steps) for s in range(steps + 1)]
    dt = length / steps
    speeds = []
    for name in frames[0]:
        ys = [f[name][1] for f in frames]
        low = min(ys) + (max(ys) - min(ys)) * 0.5
        for s in range(steps):
            moved = frames[s + 1][name][2] - frames[s][name][2]
            if ys[s] <= low and ys[s + 1] <= low and moved > 0:
                speeds.append(moved / dt)
    if not speeds:
        return None
    return sum(speeds) / len(speeds) * metres


def main():
    args = sys.argv[1:]
    if len(args) < 2:
        raise SystemExit(__doc__)
    src, out = args[0], args[1]
    clips_json = args[args.index("--clips") + 1] if "--clips" in args else None
    metres = float(args[args.index("--metres") + 1]) if "--metres" in args else 0.035
    bb, bones, uuids = read_model(src)
    names = {b["name"]: i for i, b in enumerate(bones)}
    sources = list(retargeted_clips(clips_json, names)) if clips_json else list(own_clips(bb, uuids))
    clips = {c["name"]: build_clip(c, bones) for c in sources}
    stride = {}
    for name in ("walk", "sprint"):
        if name in clips:
            speed = ground_speed(bones, clips[name], metres)
            if speed and speed > 0.05:
                stride[name] = {"ref": r5(speed), "reverse": False}
    doc = {
        "unit": r5(1.0 / metres),
        "metres": metres,
        "bones": [[b["name"], b["parent"], [r5(x) for x in b["rest"]], [0.0, 0.0, 0.0, 1.0]] for b in bones],
        "clips": clips,
        "stride": stride,
    }
    json.dump(doc, open(out, "w"), separators=(",", ":"))
    for name, c in clips.items():
        print("%-8s len %.3f loop %-5s tracks %d" % (name, c["len"], c["loop"], len(c["tracks"])))
    print("stride", stride)


if __name__ == "__main__":
    main()
