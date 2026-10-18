"""Instrument an exported ONNX model: add all Pad `pads` inputs and conv in/out
tensors as extra outputs, run with a concrete S, print shapes + pad values.

Usage: python inspect_pads.py <model.onnx> <S> [out_json]
"""
import json
import sys

import numpy as np
import onnx
import onnxruntime as ort


def main():
    path, S = sys.argv[1], int(sys.argv[2])
    out_json = sys.argv[3] if len(sys.argv) > 3 else None
    m = onnx.load(path)
    g = m.graph
    extra = []
    for n in g.node:
        if n.op_type == "Pad":
            extra.append(n.input[1])  # pads tensor
            extra.append(n.output[0])
        if n.op_type in ("Conv", "ConvTranspose"):
            extra.append(n.input[0])
            extra.append(n.output[0])
    # dedupe, keep order
    seen = set()
    extra = [e for e in extra if not (e in seen or seen.add(e))]

    info = {}
    vi_names = {v.name for v in g.value_info} | {v.name for v in g.output}
    for name in extra:
        if name not in vi_names:
            g.value_info.append(onnx.helper.ValueInfoProto(name=name))
    for name in extra:
        g.output.append(onnx.helper.ValueInfoProto(name=name))

    opts = ort.SessionOptions()
    opts.log_severity_level = 3
    sess = ort.InferenceSession(m.SerializeToString(), opts, providers=["CPUExecutionProvider"])

    feeds = {}
    for i in sess.get_inputs():
        dims = [S if (isinstance(d, str) or d is None) else d for d in i.shape]
        feeds[i.name] = np.zeros(dims, dtype=np.float32)
    outs = sess.run(None, feeds)
    names = [o.name for o in sess.get_outputs()]
    for name, val in zip(names, outs):
        if name in extra:
            info[name] = {"shape": list(val.shape), "data": val.ravel().tolist() if val.size < 64 else None}
            if val.size < 64:
                print(f"{name:55s} shape={list(val.shape)} data={val.ravel().tolist()}")
            elif val.ndim > 0 and (val.size < 5000):
                print(f"{name:55s} shape={list(val.shape)} head={val.ravel()[:12].tolist()}")
            else:
                print(f"{name:55s} shape={list(val.shape)}")
    if out_json:
        with open(out_json, "w") as f:
            json.dump(info, f, indent=1)


if __name__ == "__main__":
    main()
