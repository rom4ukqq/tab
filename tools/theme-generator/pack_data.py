from pathlib import Path
import base64
import json

root = Path(__file__).resolve().parent
payload = {"meta": json.loads((root / "frames.json").read_text(encoding="utf-8")),
           "gzip": base64.b64encode((root / "frames.bin.gz").read_bytes()).decode("ascii")}
(root / "theme-data.js").write_text("window.themeData=" + json.dumps(payload) + ";", encoding="utf-8")
