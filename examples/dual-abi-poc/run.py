from pathlib import Path
import subprocess

root = Path(__file__).resolve().parent

def run(*args):
    subprocess.run(args, cwd=root, check=True)

run("cargo", "build", "--locked", "--release", "--target", "wasm32-unknown-unknown",
    "-p", "guest-v01", "-p", "guest-v02")
(root / "components").mkdir(exist_ok=True)
for name in ["guest-v01", "guest-v02"]:
    module = root / "target/wasm32-unknown-unknown/release" / (name.replace("-", "_") + ".wasm")
    component = root / "components" / (name + ".wasm")
    run("wasm-tools", "component", "new", str(module), "-o", str(component))
    run("wasm-tools", "validate", "--features", "all", str(component))
run("cargo", "run", "--locked", "-p", "host")
