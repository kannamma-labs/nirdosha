"""
Generates crates/nirdosha-ctms/tests/fixtures/ctms_velocity_signal_v1_test_fixture.onnx
-- a tiny, real ONNX graph (MatMul + Add + Sigmoid) over exactly the two
features this crate can honestly compute from its own data (debit count and
debit total over the rule's window), mirroring
crates/nirdosha-scoring-model-onnx/tests/fixtures/rt_fraud_v1_test_fixture.onnx's
own shape (single [1, N] "features" input, single "score" output) but with a
feature set this crate actually has, rather than reusing that fixture's
mismatched fraud-specific features (velocity_1h, distinct_payees_7d,
amount_dev_30d, impossible_travel) this crate has no honest way to compute.

Reference weights only -- not a production AML model, exactly like the
existing fixture's own disclosed scope.
"""
import hashlib
import numpy as np
import onnx
from onnx import helper, TensorProto

W = np.array([[0.6], [0.9]], dtype=np.float32)  # [debit_count, debit_total_minor_norm]
B = np.array([-3.0], dtype=np.float32)

features = helper.make_tensor_value_info("features", TensorProto.FLOAT, [1, 2])
score = helper.make_tensor_value_info("score", TensorProto.FLOAT, [1, 1])

w_init = helper.make_tensor("W", TensorProto.FLOAT, [2, 1], W.flatten().tolist())
b_init = helper.make_tensor("B", TensorProto.FLOAT, [1], B.flatten().tolist())

matmul_node = helper.make_node("MatMul", ["features", "W"], ["matmul_out"])
add_node = helper.make_node("Add", ["matmul_out", "B"], ["logit"])
sigmoid_node = helper.make_node("Sigmoid", ["logit"], ["score"])

graph = helper.make_graph(
    [matmul_node, add_node, sigmoid_node],
    "ctms_velocity_signal_v1",
    [features],
    [score],
    initializer=[w_init, b_init],
)
model = helper.make_model(graph, producer_name="nirdosha-ctms-fixture-generator", opset_imports=[helper.make_opsetid("", 13)])
model.ir_version = 8
onnx.checker.check_model(model)

out_path = "crates/nirdosha-ctms/tests/fixtures/ctms_velocity_signal_v1_test_fixture.onnx"
onnx.save(model, out_path)

with open(out_path, "rb") as f:
    digest = hashlib.sha256(f.read()).hexdigest()
with open(out_path + ".sha256", "w") as f:
    f.write(digest + "\n")

print("wrote", out_path, "sha256", digest)

# Independent reference score for a test to check against, computed here
# with plain numpy (not onnxruntime) -- same convention the existing
# fixture's own doc comment uses.
def sigmoid(x):
    return 1.0 / (1.0 + np.exp(-x))

for debit_count, debit_total_norm in [(5.0, 1.0), (0.0, 0.0), (10.0, 3.0)]:
    x = np.array([debit_count, debit_total_norm], dtype=np.float32)
    logit = float(x @ W.flatten() + B[0])
    print(f"debit_count={debit_count} debit_total_norm={debit_total_norm} -> score={sigmoid(logit):.7f}")
