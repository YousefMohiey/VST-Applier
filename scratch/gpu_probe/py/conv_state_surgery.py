"""Extend the ONNX state surgery from GRU hidden states to time-axis (S-axis)
convolution delay lines.

For every Pad node in the graph that prepends L zeros along the sequence axis
(the exporters emit  L = (k_S-1)*dilation  left-context pads, computed
dynamically from Shape()):

    X --[Pad, L zeros left on S]--> Xp --[Conv k_S>1, valid on S]--> Y

we rewrite it into a streaming form:

    STATE_IN --\
                [Concat, axis=S]--> X_eff --[Conv, same attrs]--> Y
    X ---------/                    |
                                    [Slice, 1:] --> STATE_OUT

- STATE_IN carries the last L frames of the conv input (zero-initialised at
  stream start, which reproduces the Pad's zeros exactly).
- The Pad node is removed (its left-context now lives in the state).
- The conv keeps its original attributes: it was already 'valid' on S
  (no S padding in the conv itself); only the freq-axis padding stays.

Proof of equivalence (batch, state=0): conv state+input gives
  out[t] = conv(x[t-L .. t])  with zeros for t<0
which is exactly what the padded batch graph computes; for t>=L no state is
read.  Frame by frame (S=1) this is the standard streaming convolution.

Writes <out_dir>/<model>.onnx and <out_dir>/<model>.states.json.

Usage: python conv_state_surgery.py <in_dir> <out_dir> [model ...]
"""
import json
import os
import sys

import numpy as np
import onnx
import onnxruntime as ort
from onnx import TensorProto, helper

INT_MAX = 2**31 - 1


def run_instrumented(model: onnx.ModelProto, S: int = 2):
    """Run with the pad-pads tensors surfaced; return {node_name: pads(list)}."""
    g = model.graph
    want = {}
    for n in g.node:
        if n.op_type == "Pad":
            want[n.name] = n.input[1]
    names = list(want.values())
    for name in names:
        g.output.append(helper.ValueInfoProto(name=name))
    opts = ort.SessionOptions()
    opts.log_severity_level = 3
    sess = ort.InferenceSession(model.SerializeToString(), opts, providers=["CPUExecutionProvider"])
    feeds = {}
    for i in sess.get_inputs():
        dims = [S if (isinstance(d, str) or d is None) else d for d in i.shape]
        feeds[i.name] = np.zeros(dims, dtype=np.float32)
    outs = sess.run(names, feeds)
    return {node: pads.astype(np.int64).ravel().tolist() for node, pads in zip(want, outs)}


def analyze(base_path: str, S: int = 2):
    """Return a list of conv-state descriptors found in the base model."""
    m = onnx.load(base_path)
    g = m.graph
    pads_by_node = run_instrumented(onnx.load(base_path), S=S)
    consumers = {}
    for n in g.node:
        for i in n.input:
            consumers.setdefault(i, []).append(n)

    descs = []
    for n in g.node:
        if n.op_type != "Pad":
            continue
        pads = pads_by_node[n.name]
        rank = len(pads) // 2
        nz = {}
        for ax in range(rank):
            b, e = pads[ax], pads[rank + ax]
            if b or e:
                nz[ax] = (b, e)
        if not nz:
            continue
        if len(nz) != 1 or list(nz.values())[0][1] != 0 or list(nz.values())[0][0] <= 0:
            raise RuntimeError(f"{n.name}: unsupported pad pattern {pads}")
        axis = next(iter(nz))
        L = nz[axis][0]
        cons = consumers.get(n.output[0], [])
        assert len(cons) == 1, (n.name, [c.name for c in cons])
        conv = cons[0]
        if conv.op_type not in ("Conv", "ConvTranspose"):
            raise RuntimeError(f"{n.name}: consumer is {conv.op_type}")
        attrs = {a.name: helper.get_attribute_value(a) for a in conv.attribute}
        kernel = attrs["kernel_shape"]
        dil = attrs.get("dilations", [1] * len(kernel))
        assert "auto_pad" not in attrs, conv.name
        k = kernel[axis - 2]
        d = dil[axis - 2]
        assert k > 1, (conv.name, "kernel 1 on padded axis?!")
        assert L == (k - 1) * d, (conv.name, L, k, d)
        # conv must be 'valid' on the S axis itself (all left-context was in the Pad)
        conv_pads = attrs.get("pads", [0] * (2 * len(kernel)))
        sp = len(kernel)  # second half of pads = end pads
        assert conv_pads[axis - 2] == 0 and conv_pads[sp + axis - 2] == 0, (conv.name, conv_pads)
        if conv.op_type == "Conv":
            stride = attrs.get("strides", [1] * len(kernel))[axis - 2]
            assert stride == 1, (conv.name, "strided on S")
        else:
            stride = attrs.get("strides", [1] * len(kernel))[axis - 2]
            assert stride == 1, (conv.name, "ConvTranspose strided on S")
        descs.append({"pad": n.name, "conv": conv.name, "axis": axis, "L": L, "k": k,
                      "d": d, "x": n.input[0], "pad_out": n.output[0]})
    return descs


def sanitize(name: str) -> str:
    return name.lstrip("/").replace("/", "_")


def fix_value_info(g):
    produced = {i for n in g.node for i in n.output}
    produced |= {i.name for i in g.input}
    keep = [v for v in g.value_info if v.name in produced]
    del g.value_info[:]
    g.value_info.extend(keep)


def prune_dead(g):
    while True:
        consumed = {i for n in g.node for i in n.input}
        out_names = {o.name for o in g.output}
        dead = {n.name for n in g.node
                if not any(o in consumed or o in out_names for o in n.output)}
        if not dead:
            break
        keep = [n for n in g.node if n.name not in dead]
        del g.node[:]
        g.node.extend(keep)
    fix_value_info(g)


def add_conv_states(base_path: str, out_path: str, S_probe: int = 2):
    m = onnx.load(base_path)
    g = m.graph
    descs = analyze(base_path, S=S_probe)

    # shapes of the pad inputs come from a concrete run (state shape needs C/F dims)
    m_inst = onnx.load(base_path)
    inst_names = []
    for d in descs:
        inst_names += [d["x"], d["pad_out"]]
    for name in inst_names:
        m_inst.graph.output.append(helper.ValueInfoProto(name=name))
    opts = ort.SessionOptions()
    opts.log_severity_level = 3
    sess = ort.InferenceSession(m_inst.SerializeToString(), opts, providers=["CPUExecutionProvider"])
    feeds = {}
    for i in sess.get_inputs():
        dims = [S_probe if (isinstance(d, str) or d is None) else d for d in i.shape]
        feeds[i.name] = np.zeros(dims, dtype=np.float32)
    shapes = dict(zip(inst_names, sess.run(inst_names, feeds)))

    node_by_name = {n.name: n for n in g.node}
    states = []
    concats = {}   # conv name -> concat node (inserted right before the conv)
    tails = []
    for d in descs:
        conv = node_by_name[d["conv"]]
        pad_node = node_by_name[d["pad"]]
        x_shape = list(shapes[d["x"]].shape)
        base = sanitize(d["conv"])
        s_in, s_out = f"{base}_state_in", f"{base}_state_out"
        x_eff = f"{base}_x_eff"
        # state shape = conv input shape with the S axis replaced by (k-1)*d
        keep = (d["k"] - 1) * d["d"]
        st = list(x_shape)
        st[d["axis"]] = keep

        concat = helper.make_node("Concat", [s_in, pad_node.input[0]], [x_eff],
                                  axis=d["axis"], name=f"{base}_stream_concat")
        concats[d["conv"]] = concat
        assert conv.input[0] == pad_node.output[0]
        conv.input[0] = x_eff

        starts = helper.make_tensor(f"{base}_sl_starts", TensorProto.INT64, [1], [1])
        ends = helper.make_tensor(f"{base}_sl_ends", TensorProto.INT64, [1], [INT_MAX])
        axes = helper.make_tensor(f"{base}_sl_axes", TensorProto.INT64, [1], [d["axis"]])
        g.initializer.extend([starts, ends, axes])
        sl = helper.make_node("Slice", [x_eff, starts.name, ends.name, axes.name], [s_out],
                              name=f"{base}_stream_tail")
        tails.append(sl)

        g.input.append(helper.make_tensor_value_info(s_in, TensorProto.FLOAT, st))
        g.output.append(helper.make_tensor_value_info(s_out, TensorProto.FLOAT, st))
        states.append({"input": s_in, "output": s_out, "shape": st, "init": "zeros",
                       "kind": "conv_delay", "conv": d["conv"], "frames": keep})

    # rebuild the node list: original order, concat inserted before its conv,
    # pd nodes dropped, slices appended at the end.
    pad_names = {d["pad"] for d in descs}
    ordered = []
    for n in g.node:
        if n.name in pad_names:
            continue
        if n.name in concats:
            ordered.append(concats[n.name])
        ordered.append(n)
    ordered += tails
    del g.node[:]
    g.node.extend(ordered)
    prune_dead(g)

    onnx.checker.check_model(m)
    onnx.save(m, out_path)
    print(f"{os.path.basename(out_path)}: {len(states)} conv states:", flush=True)
    for s in states:
        print("   ", s["input"], s["shape"], "<-", s["conv"], flush=True)
    return states


def collect_all_states(export_path: str, model_path: str):
    """Manifest for the final model: graph-driven wiring of every state pair."""
    exp = onnx.load(export_path).graph
    act = onnx.load(model_path).graph
    exp_in = {i.name for i in exp.input}
    consumer = {}
    for n in act.node:
        for i in n.input:
            consumer.setdefault(i, []).append(n)
    sl_by_in = {n.input[0]: n.output[0] for n in act.node if n.op_type == "Slice"}
    states = []
    for i in act.input:
        if i.name in exp_in:
            continue
        cons = consumer.get(i.name, [])
        assert len(cons) == 1, (i.name, [c.name for c in cons])
        c = cons[0]
        if c.op_type == "GRU":
            out = c.output[1]
            kind, conv = "gru_hidden", c.name
        elif c.op_type == "Concat":
            out = sl_by_in[c.output[0]]
            kind, conv = "conv_delay", None
        else:
            raise RuntimeError(f"unexpected state consumer {c.op_type} for {i.name}")
        shape = [d.dim_value if d.HasField("dim_value") else str(d.dim_param)
                 for d in i.type.tensor_type.shape.dim]
        states.append({"input": i.name, "output": out, "shape": shape, "init": "zeros",
                       "kind": kind, **({"consumer": conv} if conv else {})})
        assert out in {o.name for o in act.output}, out
    return states


def main():
    in_dir, out_dir = sys.argv[1], sys.argv[2]
    models = sys.argv[3:] or ["enc", "erb_dec", "df_dec"]
    os.makedirs(out_dir, exist_ok=True)
    for name in models:
        base = os.path.join(in_dir, f"{name}.onnx")
        out = os.path.join(out_dir, f"{name}.onnx")
        add_conv_states(base, out)
        man = collect_all_states(
            os.path.join(os.path.dirname(in_dir), "export", f"{name}.onnx"), out)
        with open(os.path.join(out_dir, f"{name}.states.json"), "w") as f:
            json.dump({"model": name, "states": man}, f, indent=1)
        print(f"   manifest: {len(man)} states")


if __name__ == "__main__":
    main()
