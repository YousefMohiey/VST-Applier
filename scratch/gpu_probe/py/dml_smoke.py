"""Smoke test: do the stateful models run under the DirectML EP, and does the
DML result agree with the CPU EP over a few streamed frames?

Usage: python dml_smoke.py [model ...]   (default: enc erb_dec df_dec)
"""
import json
import os
import sys

import numpy as np
import onnxruntime as ort

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MODELS = os.path.join(ROOT, "tmp", "conv_stateful")
N = 8


def make_session(path, providers):
    opts = ort.SessionOptions()
    opts.log_severity_level = 3
    return ort.InferenceSession(path, opts, providers=providers)


def run_frames(sess, manifest, frames, n):
    inputs = {i.name: i for i in sess.get_inputs()}
    outputs = [o.name for o in sess.get_outputs()]
    so2si = {st["output"]: st["input"] for st in manifest}
    states = {}
    for st in manifest:
        states[st["input"]] = np.zeros([d if isinstance(d, int) and d > 0 else 1
                                        for d in st["shape"]], dtype=np.float32)
    acc = {}
    for t in range(n):
        feeds = dict(states)
        feeds.update({nm: frames[nm][t] for nm in frames})
        out = sess.run(None, feeds)
        for nm, val in zip(outputs, out):
            if nm in so2si:
                states[so2si[nm]] = val
            else:
                acc.setdefault(nm, []).append(val)
    return acc, states


def per_frame_shapes(model):
    if model == "enc":
        return {"feat_erb": (1, 1, 1, 32), "feat_spec": (1, 2, 1, 96)}
    if model == "erb_dec":
        return {"emb": (1, 1, 512), "e3": (1, 64, 1, 8), "e2": (1, 64, 1, 8),
                "e1": (1, 64, 1, 16), "e0": (1, 64, 1, 32)}
    return {"emb": (1, 1, 512), "c0": (1, 64, 1, 96)}


def main():
    which = sys.argv[1:] or ["enc", "erb_dec", "df_dec"]
    rng = np.random.default_rng(7)
    for model in which:
        with open(f"{MODELS}/{model}.states.json") as f:
            manifest = json.load(f)["states"]
        path = f"{MODELS}/{model}.onnx"
        cpu = make_session(path, ["CPUExecutionProvider"])
        dml = make_session(path, ["DmlExecutionProvider", "CPUExecutionProvider"])
        used = dml.get_providers()
        frames = {nm: rng.standard_normal((N,) + shp).astype(np.float32)
                  for nm, shp in per_frame_shapes(model).items()}
        acc_cpu, st_cpu = run_frames(cpu, manifest, frames, N)
        acc_dml, st_dml = run_frames(dml, manifest, frames, N)
        print(f"== {model} providers={used}")
        for nm in acc_cpu:
            a = np.concatenate([v.reshape(-1) for v in acc_cpu[nm]])
            b = np.concatenate([v.reshape(-1) for v in acc_dml[nm]])
            d = np.abs(a - b)
            print(f"   {nm:6s} n={a.size:7d} max_abs_cpu_vs_dml={d.max():.3e} "
                  f"bit-identical={np.array_equal(a, b)}")
        for si, v in st_cpu.items():
            w = st_dml[si]
            print(f"   state {si:32s} max_abs_cpu_vs_dml={np.abs(v - w).max():.3e}")


if __name__ == "__main__":
    main()
