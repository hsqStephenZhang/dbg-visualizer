"""Exercise CodeLLDB's actual DAP adapter without a graphical VS Code instance.

python3 tests/integration/codelldb.py --adapter /path/to/extension/adapter/codelldb
"""
import argparse
import json
from pathlib import Path
import socket
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]


class Dap:
    def __init__(self, connection):
        self.connection = connection
        self.file = connection.makefile("rb")
        self.sequence = 0
        self.events = []
        self.responses = {}

    def send(self, command, arguments=None):
        self.sequence += 1
        payload = json.dumps(dict(seq=self.sequence, type="request", command=command, arguments=arguments or {})).encode()
        self.connection.sendall(f"Content-Length: {len(payload)}\r\n\r\n".encode() + payload)
        return self.sequence

    def receive(self):
        header = {}
        while True:
            line = self.file.readline()
            if not line: raise RuntimeError("adapter closed the DAP connection")
            if line == b"\r\n": break
            key, value = line.decode().split(":", 1)
            header[key.lower()] = value.strip()
        message = json.loads(self.file.read(int(header["content-length"])))
        if message["type"] == "response": self.responses[message["request_seq"]] = message
        elif message["type"] == "event": self.events.append(message)
        elif message["type"] == "request": raise RuntimeError("unsupported reverse request: " + str(message))
        return message

    def response(self, sequence):
        while sequence not in self.responses: self.receive()
        response = self.responses.pop(sequence)
        if not response.get("success"):
            raise RuntimeError(str(response))
        return response.get("body", {})

    def request(self, command, arguments=None):
        return self.response(self.send(command, arguments))

    def event(self, name):
        while True:
            for i, event in enumerate(self.events):
                if event["event"] == name:
                    return self.events.pop(i).get("body", {})
            self.receive()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--adapter", required=True)
    args = parser.parse_args()
    subprocess.run(["cargo", "build", "--offline"], cwd=ROOT, check=True)
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        listener.settimeout(15)
        port = listener.getsockname()[1]
        process = subprocess.Popen([args.adapter, "--connect", str(port)], cwd=ROOT,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        try:
            connection, _ = listener.accept()
            connection.settimeout(20)
            with connection:
                dap = Dap(connection)
                dap.request("initialize", dict(adapterID="lldb", clientID="dbgvis-tests", linesStartAt1=True,
                                               columnsStartAt1=True, pathFormat="path", supportsRunInTerminalRequest=False))
                launch = dap.send("launch", dict(program=str(ROOT / "target/debug/dbg-visualizer"),
                    cwd=str(ROOT), stopOnEntry=False,
                    initCommands=[f'command script import "{ROOT}/debugger/lldb/dbgvis_lldb.py"'],
                    preRunCommands=["dbgvis-install-hook"]))
                dap.event("initialized")
                dap.request("setFunctionBreakpoints", {"breakpoints": [{"name": "dbg_visualizer::checkpoint"}]})
                dap.request("configurationDone")
                dap.response(launch)
                stopped = dap.event("stopped")
                thread = stopped["threadId"]

                def frame():
                    frames = dap.request("stackTrace", {"threadId": thread})["stackFrames"]
                    return next(f["id"] for f in frames if "dbg_visualizer::main" in f["name"])

                def evaluate(command):
                    response = dap.request("evaluate", {"expression": command, "context": "repl", "frameId": frame()})
                    return response.get("result", "")

                evaluate("dbgvis config summary.mode debug")
                evaluate("dbgvis config execution automatic")
                evaluate("dbgvis config children.mode structured")
                evaluate("dbgvis config children.page_size 2")
                evaluate("dbgvis config children.max_items 3")
                scopes = dap.request("scopes", {"frameId": frame()})["scopes"]
                locals_ref = next(scope["variablesReference"] for scope in scopes if "local" in scope["name"].lower())
                variables = dap.request("variables", {"variablesReference": locals_ref})["variables"]
                by_name = {item["name"]: item for item in variables}
                assert 'b"hello world"' in by_name["bytes"]["value"], by_name
                assert "临时字符串" in by_name["map"]["value"], by_name
                children = dap.request("variables", {"variablesReference": by_name["map"]["variablesReference"]})["variables"]
                assert any(child["name"] == "[0].key" and child["value"] == "1" for child in children), children
                assert any(child["name"] == "[1].value" and "临时字符串" in child["value"] for child in children), children
                large_children = dap.request("variables", {"variablesReference": by_name["large"]["variablesReference"]})["variables"]
                assert any(child["name"] == "..." and "9997 more items" in child["value"] for child in large_children), large_children
                evaluate("dbgvis config --type dbg_visualizer::Point summary.mode display")
                scopes = dap.request("scopes", {"frameId": frame()})["scopes"]
                local = next(scope["variablesReference"] for scope in scopes if "local" in scope["name"].lower())
                variables = dap.request("variables", {"variablesReference": local})["variables"]
                assert any(item["name"] == "point" and "(3, 7)" in item["value"] for item in variables), variables
                dap.request("continue", {"threadId": thread})
                dap.event("stopped")
                scopes = dap.request("scopes", {"frameId": frame()})["scopes"]
                local = next(scope["variablesReference"] for scope in scopes if "local" in scope["name"].lower())
                variables = dap.request("variables", {"variablesReference": local})["variables"]
                text = next(item["value"] for item in variables if item["name"] == "map")
                assert '4: "four"' in text and '1: "one"' not in text, text
                dap.request("disconnect", {"terminateDebuggee": True})
                print("CODELLDB_DAP_OK")
        finally:
            if process.poll() is None: process.terminate()
            output, _ = process.communicate(timeout=10)
            if output.strip(): print(output)


if __name__ == "__main__": main()
