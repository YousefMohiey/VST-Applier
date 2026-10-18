"""Validate stateful ORT models against tract's pulsed ground-truth dumps.

Two modes per model:
  * batch sanity: run the stateful model once with S=64 and all states zero;
    must equal the tract dumps (the conv surgery keeps batch semantics).
  * streaming: run S=1 64 times, carrying every state output into the matching
    state input; must equal the tract dumps.

Usage: python validate_stream.py enc [erb_dec] [df_dec]
"""
import os
import sys

import numpy as np
import onnxruntime as ort

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MODELS = os.path.join(ROOT, "tmp", "conv_stateful")
NFRAMES = 64


def load_dump(name):
    for d in ("pulse_out2", "pulse_out"):
        p = f"{ROOT}/{d}/{name}.raw"
        if os.path.exists(p):
            return np.fromfile(p, dtype="<f4")
    raise FileNotFoundError(name)


def session(path):
    opts = ort.SessionOptions()
    opts.log_severity_level = 3
    return ort.InferenceSession(path, opts, providers=["CPUExecutionProvider"])


def stats(a, b):
    d = np.abs(a - b)
    scale = max(np.max(np.abs(b)), 1e-12)
    mask = np.abs(b) > 1e-3
    rel = (d[mask] / np.abs(b[mask])).max() if mask.any() else 0.0
    return dict(n=a.size, max_abs=float(d.max()), max_abs_over_scale=float(d.max() / scale),
                max_rel_gt_1m3=float(rel), mean_abs=float(d.mean()),
                n_gt_1m3=int((d > 1e-3).sum()), n_gt_1m4=int((d > 1e-4).sum()))


def report(tag, name, a, b):
    s = stats(a, b)
    print(f"  {tag:10s} {name:5s} n={s['n']:7d} max_abs={s['max_abs']:.3e} "
          f"max_abs/scale={s['max_abs_over_scale']:.3e} max_rel(|t|>1e-3)={s['max_rel_gt_1m3']:.3e} "
          f"mean_abs={s['mean_abs']:.3e} n>1e-3:{s['n_gt_1m3']} n>1e-4:{s['n_gt_1m4']}")
    return s


def batch_flat(name, val):
    """Batch output -> the flat frame-major layout of the tract dumps."""
    v = np.asarray(val)
    if name in ("e0", "e1", "e2", "e3", "c0", "m"):
        return v[0].transpose(1, 0, 2).reshape(-1)   # [1,C,S,F] -> [S,C,F]
    if name == "lsnr":
        return v[0].reshape(-1)
    return v.reshape(-1)                              # emb [1,S,512], coefs...


def batch_sanity(model, dump_names, feed_frames):
    """Run the stateful model with S=64, states=0, compare against tract dumps."""
    sess = session(f"{MODELS}/{model}.onnx")
    inputs = {i.name: i for i in sess.get_inputs()}
    feeds = dict(feed_frames)
    for nm, i in inputs.items():
        if nm in feeds:
            continue
        shape = [d if isinstance(d, int) and d > 0 else NFRAMES if d in ("S",) else 1
                 for d in i.shape]
        feeds[nm] = np.zeros(shape, dtype=np.float32)
    out = sess.run(None, feeds)
    names = [o.name for o in sess.get_outputs()]
    res = {}
    flats = {}
    for nm, val in zip(names, out):
        if nm in dump_names:
            flat = batch_flat(nm, val)
            res[nm] = report(f"{model}/batch", nm, flat, load_dump(dump_names[nm]))
            flats[nm] = flat
    return flats


def zero_states(model):
    import json
    with open(f"{MODELS}/{model}.states.json") as f:
        manifest = json.load(f)["states"]
    states = {}
    for st in manifest:
        states[st["input"]] = np.zeros([d if isinstance(d, int) and d > 0 else 1
                                        for d in st["shape"]], dtype=np.float32)
    return states


def stream(model, dump_names, input_frames, n=None, init_states=None):
    """S=1 loop carrying all states (wired from the states.json manifest)."""
    import json
    sess = session(f"{MODELS}/{model}.onnx")
    inputs = {i.name: i for i in sess.get_inputs()}
    outputs = [o.name for o in sess.get_outputs()]
    with open(f"{MODELS}/{model}.states.json") as f:
        manifest = json.load(f)["states"]
    so2si = {st["output"]: st["input"] for st in manifest}
    states = zero_states(model) if init_states is None else {k: v.copy() for k, v in init_states.items()}
    if n is None:
        n = len(next(iter(input_frames.values())))
    missing = [st["input"] for st in manifest if st["input"] not in inputs]
    assert not missing, f"manifest inputs missing from graph: {missing}"
    assert all(st["output"] in outputs for st in manifest), \
        f"manifest outputs missing: {[s['output'] for s in manifest if s['output'] not in outputs]}"

    acc = {}
    for t in range(n):
        feeds = dict(states)
        for nm, frames in input_frames.items():
            feeds[nm] = frames[t]
        out = sess.run(None, feeds)
        for nm, val in zip(outputs, out):
            if nm in so2si:
                states[so2si[nm]] = val
            else:
                acc.setdefault(nm, []).append(val.reshape(-1))
    res = {}
    flats = {}
    for nm, chunks in acc.items():
        flat = np.concatenate(chunks)
        flats[nm] = flat
        if nm in dump_names:
            res[nm] = report(f"{model}/stream", nm, flat, load_dump(dump_names[nm]))
        else:
            res[nm] = flat  # e.g. enc intermediates, no dump to compare
    return flats, res, states


def chained(model, frames):
    """Chunking invariance: 128-frame stream vs 64+64 with carried states."""
    frames2 = {nm: np.concatenate([f, f], axis=0) for nm, f in frames.items()}
    f2, _, _ = stream(model, {}, frames2)
    fa, _, sa = stream(model, {}, frames)
    fb, _, _ = stream(model, {}, frames, init_states=sa)
    ok_a = all(np.array_equal(f2[nm][:fa[nm].size], fa[nm]) for nm in fa if nm in f2)
    ok_b = all(np.array_equal(f2[nm][fa[nm].size:], fb[nm]) for nm in fb if nm in f2)
    print(f"  {model} chunking invariance (128-stream vs 64+64): "
          f"first-half bit-identical={ok_a}, second-half bit-identical={ok_b}")


def frames_from_flow(enc_outs, layout):
    """Per-frame feeds from streamed enc outputs; layout = per-frame shape w/o batch."""
    out = {}
    for nm, shape in layout.items():
        per = int(np.prod(shape))
        arr = enc_outs[nm].reshape(NFRAMES, per).reshape(NFRAMES, *shape)
        out[nm] = np.expand_dims(arr, 1)
    return out


def enc_case():
    erb = load_dump("in_feat_erb").reshape(NFRAMES, 32)
    spec = load_dump("in_feat_spec").reshape(NFRAMES, 2, 96)
    dump_names = {k: f"tract_{k}" for k in ["e0", "e1", "e2", "e3", "emb", "c0", "lsnr"]}
    frames_t = {"feat_erb": np.expand_dims(erb, (1, 2, 3)).astype(np.float32),
                "feat_spec": np.expand_dims(spec, (1, 3)).astype(np.float32)}
    batch = batch_sanity("enc", dump_names,
                         {"feat_erb": erb[None, None], "feat_spec": spec.transpose(1, 0, 2)[None]})
    flats, _, _ = stream("enc", dump_names, frames_t)
    for nm in batch:
        print(f"  enc stream vs batch {nm}: bit-identical={np.array_equal(batch[nm], flats[nm])}")
    return flats


def dump_frames(prefix, shapes):
    """Per-frame inputs straight from the tract dumps (decoder isolation test)."""
    out = {}
    for nm, shape in shapes.items():
        per = int(np.prod(shape))
        arr = load_dump(f"{prefix}_{nm}").reshape(NFRAMES, per).reshape(NFRAMES, *shape)
        out[nm] = np.expand_dims(arr, 1)
    return out


def batch_feeds(enc_outs, layout):
    """Batch (S=64) feeds from streamed enc outputs; layout per-frame shape w/o batch."""
    feeds = {"emb": enc_outs["emb"].reshape(NFRAMES, 512)[None]}   # [1,S,512]
    for nm in ("e3", "e2", "e1", "e0", "c0"):
        if nm in layout:
            c, s_ph, f = layout[nm]      # [C, S-placeholder(1), F]
            assert s_ph == 1, layout[nm]
            arr = enc_outs[nm].reshape(NFRAMES, c, s_ph, f)[:, :, 0, :]   # [S,C,F]
            feeds[nm] = np.transpose(arr, (1, 0, 2))[None]                # [1,C,S,F]
    return feeds


if __name__ == "__main__":
    which = sys.argv[1:] or ["enc"]
    enc_outs = None
    if "enc" in which:
        print("== enc")
        enc_outs = enc_case()
        erb = load_dump("in_feat_erb").reshape(NFRAMES, 32)
        spec = load_dump("in_feat_spec").reshape(NFRAMES, 2, 96)
        chained("enc", {"feat_erb": np.expand_dims(erb, (1, 2, 3)).astype(np.float32),
                        "feat_spec": np.expand_dims(spec, (1, 3)).astype(np.float32)})
    if "erb_dec" in which:
        print("== erb_dec")
        if enc_outs is None:
            raise SystemExit("erb_dec needs enc first (same process order)")
        in_layout = {"emb": (1, 512), "e3": (64, 1, 8), "e2": (64, 1, 8),
                     "e1": (64, 1, 16), "e0": (64, 1, 32)}
        frames = frames_from_flow(enc_outs, in_layout)
        batch = batch_sanity("erb_dec", {"m": "tract_m"}, batch_feeds(enc_outs, in_layout))
        flats, _, _ = stream("erb_dec", {"m": "tract_m"}, frames)
        print(f"  erb_dec stream vs batch m: bit-identical={np.array_equal(batch['m'], flats['m'])}")
        print("  [isolation: exact tract-fed decoder inputs]")
        iso = dump_frames("decin", in_layout)
        stream("erb_dec", {"m": "tract_m"}, iso)
        chained("erb_dec", {nm: f for nm, f in iso.items()
                            if nm in ("emb", "e3", "e2", "e1", "e0")})
    if "df_dec" in which:
        print("== df_dec")
        if enc_outs is None:
            raise SystemExit("df_dec needs enc first (same process order)")
        in_layout = {"emb": (1, 512), "c0": (64, 1, 96)}
        frames = frames_from_flow(enc_outs, in_layout)
        batch = batch_sanity("df_dec", {"coefs": "tract_coefs"}, batch_feeds(enc_outs, in_layout))
        flats, _, _ = stream("df_dec", {"coefs": "tract_coefs"}, frames)
        print("  df_dec stream vs batch coefs: "
              f"bit-identical={np.array_equal(batch['coefs'], flats['coefs'])}")
        print("  [isolation: exact tract-fed decoder inputs]")
        iso = dump_frames("decin", in_layout)
        stream("df_dec", {"coefs": "tract_coefs"}, iso)
        chained("df_dec", {nm: f for nm, f in iso.items() if nm in ("emb", "c0")})
