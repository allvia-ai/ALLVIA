#!/usr/bin/env python3
from __future__ import annotations

import html
import os
import subprocess
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


ROOT_DIR = Path(__file__).resolve().parent.parent
SCRIPTS_DIR = ROOT_DIR / "scripts"
RUNNER = SCRIPTS_DIR / "run_nl_bg.sh"
CTL = SCRIPTS_DIR / "run_nl_bg_ctl.sh"


def run_cmd(args: list[str], timeout_sec: float = 5.0) -> tuple[int, str]:
    try:
        proc = subprocess.run(args, capture_output=True, text=True, timeout=timeout_sec)
        out = (proc.stdout or "") + (proc.stderr or "")
        return proc.returncode, out.strip()
    except subprocess.TimeoutExpired:
        return 124, f"timeout after {timeout_sec:.1f}s: {' '.join(args)}"
    except Exception as e:
        return 125, f"error: {e}"


def latest_run_id() -> str:
    latest = ROOT_DIR / "scenario_results" / "bg_runs" / "latest_run_id"
    if latest.exists():
        return latest.read_text(encoding="utf-8").strip()
    return ""


def render_page(message: str = "", request_text: str = "", task_name: str = "") -> str:
    rid = latest_run_id()
    status_out = ""
    tail_out = ""
    if rid:
        _, status_out = run_cmd([str(CTL), "status", rid])
        _, tail_out = run_cmd([str(CTL), "tail", rid])

    msg = f"<pre>{html.escape(message)}</pre>" if message else ""
    return f"""<!doctype html>
<html lang="ko">
<head>
  <meta charset="utf-8" />
  <title>Steer BG Control</title>
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <style>
    :root {{
      --bg: #0f172a;
      --panel: #111827;
      --soft: #1f2937;
      --text: #e5e7eb;
      --muted: #9ca3af;
      --accent: #22c55e;
      --warn: #f59e0b;
      --stop: #ef4444;
    }}
    body {{
      margin: 0; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      background: radial-gradient(circle at top, #1e293b, var(--bg));
      color: var(--text);
    }}
    .wrap {{ max-width: 980px; margin: 24px auto; padding: 0 16px; }}
    .card {{
      background: linear-gradient(180deg, #111827, #0b1220);
      border: 1px solid #243047;
      border-radius: 12px;
      padding: 16px;
      margin-bottom: 12px;
    }}
    h1 {{ margin: 0 0 12px 0; font-size: 20px; }}
    label {{ display: block; margin-bottom: 6px; color: var(--muted); font-size: 13px; }}
    input, textarea {{
      width: 100%; box-sizing: border-box; border-radius: 8px;
      border: 1px solid #334155; background: #0b1220; color: var(--text);
      padding: 10px; margin-bottom: 10px;
    }}
    textarea {{ min-height: 120px; resize: vertical; }}
    .row {{ display: flex; gap: 8px; flex-wrap: wrap; }}
    button {{
      border: 0; border-radius: 8px; padding: 10px 14px; font-weight: 600; cursor: pointer;
      background: #334155; color: var(--text);
    }}
    .start {{ background: var(--accent); color: #06210e; }}
    .pause, .resume {{ background: var(--warn); color: #281400; }}
    .stop {{ background: var(--stop); color: #2c0909; }}
    pre {{
      white-space: pre-wrap; word-break: break-word; background: #020617;
      border: 1px solid #1f2937; border-radius: 8px; padding: 10px; margin: 8px 0 0 0;
    }}
    .muted {{ color: var(--muted); font-size: 12px; }}
  </style>
</head>
<body>
  <div class="wrap">
    <div class="card">
      <h1>Steer Background Runner</h1>
      <div class="muted">localhost only. 버튼으로 시작/일시정지/재개/중지.</div>
      {msg}
    </div>

    <div class="card">
      <form method="POST" action="/start">
        <label>요청문</label>
        <textarea name="request_text" required>{html.escape(request_text)}</textarea>
        <label>작업명</label>
        <input name="task_name" value="{html.escape(task_name or '백그라운드 실행')}" />
        <div class="row">
          <button class="start" type="submit">Start Run</button>
        </div>
      </form>
    </div>

    <div class="card">
      <div class="row">
        <form method="POST" action="/pause"><button class="pause" type="submit">Pause</button></form>
        <form method="POST" action="/resume"><button class="resume" type="submit">Resume</button></form>
        <form method="POST" action="/stop"><button class="stop" type="submit">Stop</button></form>
        <form method="POST" action="/refresh"><button type="submit">Refresh</button></form>
      </div>
      <div class="muted">latest run id: {html.escape(rid or '(none)')}</div>
    </div>

    <div class="card">
      <label>Status</label>
      <pre>{html.escape(status_out or '(no status)')}</pre>
    </div>

    <div class="card">
      <label>Driver Log Tail</label>
      <pre>{html.escape(tail_out or '(no log)')}</pre>
    </div>
  </div>
</body>
</html>
"""


class Handler(BaseHTTPRequestHandler):
    def send_html(self, body: str, status: int = 200) -> None:
        encoded = body.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def do_GET(self) -> None:
        if self.path != "/":
            self.send_error(404, "Not Found")
            return
        body = render_page()
        self.send_html(body, 200)

    def do_POST(self) -> None:
        n = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(n).decode("utf-8")
        form = urllib.parse.parse_qs(raw)
        msg = ""
        request_text = form.get("request_text", [""])[0]
        task_name = form.get("task_name", [""])[0]
        rid = latest_run_id()

        if self.path == "/start":
            if not request_text.strip():
                msg = "request_text is required."
            else:
                code, out = run_cmd([str(RUNNER), request_text, task_name or "백그라운드 실행"])
                msg = f"[start exit={code}]\n{out}"
        elif self.path == "/pause":
            args = [str(CTL), "pause"]
            if rid:
                args.append(rid)
            code, out = run_cmd(args)
            msg = f"[pause exit={code}]\n{out}"
        elif self.path == "/resume":
            args = [str(CTL), "resume"]
            if rid:
                args.append(rid)
            code, out = run_cmd(args)
            msg = f"[resume exit={code}]\n{out}"
        elif self.path == "/stop":
            args = [str(CTL), "stop"]
            if rid:
                args.append(rid)
            code, out = run_cmd(args)
            msg = f"[stop exit={code}]\n{out}"
        elif self.path == "/refresh":
            msg = "refreshed"
        else:
            self.send_error(404, "Not Found")
            return

        body = render_page(msg, request_text, task_name)
        self.send_html(body, 200)

    def log_message(self, fmt: str, *args) -> None:
        return


def main() -> None:
    host = os.environ.get("STEER_BG_PANEL_HOST", "127.0.0.1")
    port = int(os.environ.get("STEER_BG_PANEL_PORT", "8787"))
    server = ThreadingHTTPServer((host, port), Handler)
    print(f"Steer BG panel listening on http://{host}:{port}")
    server.serve_forever()


if __name__ == "__main__":
    main()
