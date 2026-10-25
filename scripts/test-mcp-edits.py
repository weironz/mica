"""Exercise a built CLI over real stdio against an isolated API/Postgres.

DATABASE_URL must point at a disposable test database. No production deployment
or installed CLI is used. Both child processes and the temporary config close
on exit; the database remains available for inspection.
"""

import argparse
import json
import os
from pathlib import Path
import queue
import secrets
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
import urllib.parse


def http(base, path, body=None):
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(
        base + path, data=data, headers={"Content-Type": "application/json"}
    )
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(request, timeout=10) as response:
        return json.load(response)


class Session:
    def __init__(self, binary, config, log):
        # Model a launcher that filters HOME/APPDATA out of its child env.
        env = {key: value for key, value in os.environ.items()
               if key.upper() in {"PATH", "SYSTEMROOT", "TEMP", "TMP"}}
        env["MICA_CONFIG"] = str(config)
        self.child = subprocess.Popen(
            [str(binary), "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=log, text=True, encoding="utf-8", env=env,
        )
        self.messages = queue.Queue()
        self.next_id = 0
        threading.Thread(target=self.read, daemon=True).start()
        self.request("initialize", {
            "protocolVersion": "2025-03-26", "capabilities": {},
            "clientInfo": {"name": "mica-edit-smoke", "version": "1"},
        })
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def read(self):
        for line in self.child.stdout:
            self.messages.put(json.loads(line))

    def send(self, value):
        self.child.stdin.write(json.dumps(value) + "\n")
        self.child.stdin.flush()

    def request(self, method, params):
        self.next_id += 1
        expected = self.next_id
        self.send({"jsonrpc": "2.0", "id": expected, "method": method, "params": params})
        deadline = time.monotonic() + 30
        while True:
            message = self.messages.get(timeout=max(0.01, deadline - time.monotonic()))
            if message.get("id") == expected:
                assert "error" not in message, message
                return message["result"]

    def call(self, name, args, error=False):
        result = self.request("tools/call", {"name": name, "arguments": args})
        assert bool(result.get("isError")) == error, result
        text = "\n".join(item["text"] for item in result["content"] if item["type"] == "text")
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            return text

    def close(self):
        if self.child.poll() is None:
            self.child.stdin.close()
            try:
                self.child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.child.terminate()
                self.child.wait(timeout=5)


def exercise(session):
    tools = session.request("tools/list", {})["tools"]
    schema = next(tool for tool in tools if tool["name"] == "mica_update_document")["inputSchema"]
    assert "replacement_format" in schema["properties"]
    workspace = session.call("mica_create_workspace", {"name": "MCP edit smoke"})
    workspace_id = workspace["id"] if "id" in workspace else workspace["workspace"]["id"]
    try:
        created = session.call("mica_create_document", {
            "workspace_id": workspace_id, "name": "MCP edit regression",
            "markdown": "# Setup\n\nrun `omp --version`, confirmed\n\n"
                        "- **pending**: account login\n\n"
                        "1. one\n2. two\n3. three\n4. four\n5. five\n\n"
                        "| field | value |\n| - | - |\n| kind | full executable |\n"
                        "| duplicate | repeated |\n\nrepeated\n\n"
                        "replace_marker\n\nliteral_marker\n\n```sh\ncode_marker\n```",
        })
        doc = {"workspace_id": workspace_id, "document_id": created["document_id"]}
        read = lambda: session.call("mica_read_document", {**doc, "refresh": True})
        text = read()
        assert "1. one\n2. two\n3. three\n4. four\n5. five" in text, text
        outline = session.call("mica_get_outline", doc)
        assert [block["block_id"] for block in outline["blocks"]] == outline["block_ids"]
        assert any(block["type"] == "table" and block["preview"] for block in outline["blocks"])
        original_ids = outline["block_ids"]
        cases = [
            ("run `omp --version`, confirmed", "run `omp --help`, confirmed", {}),
            ("- **pending**: account login", "**verified**: account login", {}),
            ("full executable", "`actual_binary`", {}),
            ("replace_marker", "`DEEPSEEK_API_KEY`", {}),
            ("literal_marker", "`literal_key`", {"replacement_format": "plain_text"}),
            ("code_marker", "`raw_key`", {}),
        ]
        for find, replace, extra in cases:
            session.call("mica_update_document", {
                **doc, "mode": "find_replace", "find": find, "replace": replace,
                "unique": True, **extra,
            })
        text = read()
        for expected in ["`omp --help`", "**verified**", "`actual_binary`",
                         "`DEEPSEEK_API_KEY`", "\\`literal\\_key\\`", "```sh\n`raw_key`\n```"]:
            assert expected in text, (expected, text)
        assert session.call("mica_get_outline", doc)["block_ids"] == original_ids
        before = text
        error = session.call("mica_update_document", {
            **doc, "mode": "find_replace", "find": "repeated", "replace": "wrong", "unique": True,
        }, error=True)
        assert "2 time(s)" in str(error), error
        assert read() == before
        stale = session.call("mica_get_outline", doc)["seq"]
        session.call("mica_update_document", {**doc, "mode": "append", "markdown": "changed"})
        session.call("mica_update_document", {
            **doc, "mode": "append", "markdown": "must not land", "expected_seq": stale,
        }, error=True)
        assert "must not land" not in read()
        partial = session.call("mica_apply_edits", {
            **doc, "edits": [
                {"mode": "append", "markdown": "batch landed"},
                {"mode": "delete", "anchor": "block_missing"},
                {"mode": "append", "markdown": "must not run"},
            ],
        }, error=True)
        assert partial["applied"] == 1 and partial["failed_index"] == 1, partial
        assert partial["edits"][0]["block_ids"] and partial["seq"], partial
        text = read()
        assert "batch landed" in text and "must not run" not in text, text
        full = session.call("mica_apply_edits", {
            **doc, "edits": [
                {"mode": "append", "markdown": "batch first"},
                {"mode": "append", "markdown": "batch second"},
            ],
        })
        assert full["applied"] == 2 and all(step["block_ids"] for step in full["edits"]), full
        print(json.dumps({
            "ok": True, "transport": "real stdio -> API -> PostgreSQL/CRDT",
            "cases": ["filtered-env config", "tool schema", "ordered export", "labeled outline",
                      "inline/list/table matching", "Markdown and literal replacement",
                      "literal code", "stable block ids", "atomic unique", "stale seq",
                      "partial batch and inserted ids", "complete batch"],
        }))
    finally:
        session.call("mica_delete_workspace", {
            "workspace_id": workspace_id, "expect_name": "MCP edit smoke", "confirm": True,
        })


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--api", type=Path, required=True)
    args = parser.parse_args()
    database = os.environ["DATABASE_URL"]
    target = urllib.parse.urlsplit(database)
    assert target.hostname in {"127.0.0.1", "localhost", "::1"} and target.path.startswith("/mica_mcp_"), \
        "DATABASE_URL must name a local disposable database with the mica_mcp_ prefix"
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    password = secrets.token_urlsafe(24)
    email = "mcp-smoke@mica.test"
    env = {**os.environ, "APP_ENV": "test", "HTTP_ADDR": f"127.0.0.1:{port}",
           "DATABASE_URL": database, "MICA_SEED_TEST_USER": f"{email}:{password}",
           "JWT_SECRET": secrets.token_hex(32)}
    with tempfile.TemporaryDirectory(prefix="mica-mcp-edits-") as directory:
        directory = Path(directory)
        with (directory / "api.log").open("w", encoding="utf-8") as api_log, \
             (directory / "mcp.log").open("w", encoding="utf-8") as mcp_log:
            api = subprocess.Popen([str(args.api.resolve())], env=env, stdout=api_log, stderr=api_log)
            session = None
            try:
                deadline = time.monotonic() + 90
                while True:
                    if api.poll() is not None:
                        api_log.flush()
                        tail = (directory / "api.log").read_text(encoding="utf-8", errors="replace")[-3000:]
                        raise AssertionError(f"test API exited:\n{tail}")
                    try:
                        http(base, "/api/health")
                        break
                    except (urllib.error.URLError, TimeoutError):
                        assert time.monotonic() < deadline, "test API did not become healthy"
                        time.sleep(0.2)
                login = http(base, "/api/auth/login", {"email": email, "password": password})
                config = directory / "config.json"
                config.write_text(json.dumps({"server": base, "token": login["access_token"]}), encoding="utf-8")
                session = Session(args.cli.resolve(), config, mcp_log)
                exercise(session)
            finally:
                if session:
                    session.close()
                api.terminate()
                api.wait(timeout=10)


if __name__ == "__main__":
    main()
