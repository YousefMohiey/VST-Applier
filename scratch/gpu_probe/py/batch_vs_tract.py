"""Compare tract pulsed dumps against a plain batch ORT run of the export model.

If they match, streaming == batch-with-zero-left-context, and the conv surgery
just has to reproduce the batch semantics frame-by-frame.

Usage: python batch_vs_tract.py
"""
import numpy as np
import onnxruntime as ort

import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

erb = np.fromfile(f"{ROOT}/pulse_out/in_feat_erb.raw", dtype="<f4").reshape(64, 32)
spec = np.fromfile(f"{ROOT}/pulse_out/in_feat_spec.raw", dtype="<f4").reshape(64, 2, 96)

sess = ort.InferenceSession(f"{ROOT}/tmp/export/enc.onnx", providers=["CPUExecutionProvider"])
outs = sess.run(None, {"feat_erb": erb[None, None], "feat_spec": spec.transpose(1, 0, 2)[None]})
names = [o.name for o in sess.get_outputs()]
print("outputs:", names, [o.shape for o in outs])

# how to turn each batch output into the flat per-frame layout of the dumps
to_frames = {
    "e0": lambda v: v[0].transpose(1, 0, 2),      # [1,64,S,32] -> [S,64,32]
    "e1": lambda v: v[0].transpose(1, 0, 2),
    "e2": lambda v: v[0].transpose(1, 0, 2),
    "e3": lambda v: v[0].transpose(1, 0, 2),
    "emb": lambda v: v[0],                         # [1,S,512]
    "c0": lambda v: v[0].transpose(1, 0, 2),      # [1,64,S,96] -> [S,64,96]
    "lsnr": lambda v: v[0].reshape(64, 1),         # [1,S,1]
}
for name, val in zip(names, outs):
    a = to_frames[name](val).reshape(-1)
    b = np.fromfile(f"{ROOT}/pulse_out/tract_{name}.raw", dtype="<f4")
    assert a.size == b.size, (name, a.size, b.size)
    d = np.abs(a - b)
    denom = np.maximum(np.abs(b), 1e-6)
    print(f"{name:4s}: n={a.size:7d}  max_abs={d.max():.6g}  max_rel={np.max(d / denom):.6g}  "
          f"mean_abs={d.mean():.3g}")
