"""Local synthetic model/MCP fixture. Inspect only tool names and known test results."""
import http.server
import json
import threading

TOOLS = ["memory_search", "memory_list", "memory_read", "memory_save", "memory_edit", "memory_forget"]
PRIVATE_TEXT = "Synthetic private memory."
SHARED_TEXT = "Synthetic shared proposal."
OPERATIONS = [
    ("memory_save", {"scope": "agent", "kind": "fact", "text": PRIVATE_TEXT}),
    ("memory_read", {"scope": "agent", "path": "items/synthetic.md"}),
    ("memory_save", {"scope": "global", "kind": "fact", "text": SHARED_TEXT}),
]
EXPECTED_RESULTS = [
    "Saved synthetic-private",
    "version synthetic\n" + PRIVATE_TEXT,
    "Proposed synthetic-shared. Status: pending. User approval is required; memory has not changed.",
]


def text_content(value):
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return "\n".join(part.get("text", "") for part in value if isinstance(part, dict))
    return ""


class MemoryExecution:
    def __init__(self, provider):
        self.provider = provider
        self.finished = threading.Event()
        self.receipts = []
        self.results = {}
        self.private = {}
        self.shared = {}
        self.proposals = []
        self.tool_surfaces = []
        self.unexpected_mcp = []
        self.requests = 0

    def __enter__(self):
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def json_reply(self, value):
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(json.dumps(value).encode())

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))))
                if self.path == "/forbidden":
                    fixture.unexpected_mcp.append(self.path)
                    self.send_response(500)
                    self.end_headers()
                    return
                if self.path == "/mcp":
                    method = body.get("method", "")
                    if method.startswith("notifications/"):
                        self.send_response(202)
                        self.end_headers()
                        return
                    result = fixture.mcp_result(method, body.get("params") or {})
                    self.json_reply({"jsonrpc": "2.0", "id": body.get("id"), "result": result})
                    return
                if self.path.endswith("/count_tokens"):
                    self.json_reply({"input_tokens": 1})
                    return
                events = fixture.model_response(body)
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                for value in events:
                    event = "event: " + value["type"] + "\n" if fixture.provider == "claude" else ""
                    self.wfile.write((event + "data: " + json.dumps(value) + "\n\n").encode())

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        return self

    @property
    def port(self):
        return self.server.server_port

    def __exit__(self, *_args):
        self.server.shutdown()
        self.server.server_close()

    def mcp_result(self, method, params):
        if method == "initialize":
            return {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}}, "serverInfo": {"name": "synthetic-memory", "version": "1"}}
        if method == "tools/list":
            properties = {
                "scope": {"type": "string", "enum": ["agent", "agent-project", "global", "project"]},
                "kind": {"type": "string"}, "text": {"type": "string"}, "path": {"type": "string"},
            }
            # No read-only annotation: match the real memory server's approval requirements.
            return {"tools": [{"name": name, "description": "Synthetic scoped memory.", "inputSchema": {"type": "object", "properties": properties}} for name in TOOLS]}
        if method == "tools/call":
            name, arguments = params["name"], params.get("arguments", {})
            self.receipts.append((name, arguments))
            if (name, arguments) == OPERATIONS[0]:
                self.private["items/synthetic.md"] = arguments["text"]
                text = EXPECTED_RESULTS[0]
            elif (name, arguments) == OPERATIONS[1]:
                text = "version synthetic\n" + self.private.get(arguments["path"], "MISSING PRIVATE SAVE")
            elif (name, arguments) == OPERATIONS[2]:
                self.proposals.append({"scope": "global", "text": arguments["text"], "status": "pending"})
                text = EXPECTED_RESULTS[2]
            else:
                return {"content": [{"type": "text", "text": "Unexpected synthetic operation"}], "isError": True}
            return {"content": [{"type": "text", "text": text}], "isError": False}
        if method == "resources/list":
            return {"resources": []}
        if method == "resources/templates/list":
            return {"resourceTemplates": []}
        return {}

    def model_response(self, body):
        # Never retain system/developer text or unrelated message content.
        tools = body.get("tools") or [tool for item in body.get("input", []) for tool in item.get("tools", [])]
        names = []
        for tool in tools:
            if tool.get("type") == "namespace":
                names.extend(tool.get("name", "") + "." + child.get("name", "") for child in tool.get("tools", []))
            else:
                names.append(tool.get("name", tool.get("type")))
        self.tool_surfaces.append(set(names))
        if self.provider == "claude":
            for message in body.get("messages", []):
                content = message.get("content")
                for part in content if isinstance(content, list) else []:
                    call_id = part.get("tool_use_id", "")
                    if part.get("type") == "tool_result" and call_id in {"synthetic_0", "synthetic_1", "synthetic_2"}:
                        self.results[call_id] = text_content(part.get("content"))
        else:
            for item in body.get("input", []):
                call_id = item.get("call_id", "")
                if item.get("type") == "function_call_output" and call_id in {"synthetic_0", "synthetic_1", "synthetic_2"}:
                    self.results[call_id] = text_content(item.get("output"))
        stage = self.requests
        self.requests += 1
        if stage >= len(OPERATIONS):
            self.finished.set()
        if self.provider == "claude":
            return self.claude_response(body.get("model"), stage)
        events = [{"type": "response.created", "response": {"id": "synthetic_response_" + str(stage), "status": "in_progress"}}]
        if stage < len(OPERATIONS):
            name, arguments = OPERATIONS[stage]
            events.append({"type": "response.output_item.done", "item": {"type": "function_call", "call_id": "synthetic_" + str(stage), "namespace": "mcp__discard_memory", "name": name, "arguments": json.dumps(arguments)}})
        events.append({"type": "response.completed", "response": {"id": "synthetic_response_" + str(stage), "status": "completed", "output": [], "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}})
        return events

    def claude_response(self, model, stage):
        events = [{"type": "message_start", "message": {"id": "synthetic_response_" + str(stage), "type": "message", "role": "assistant", "model": model, "content": [], "stop_reason": None, "stop_sequence": None, "usage": {"input_tokens": 1, "output_tokens": 1}}}]
        if stage < len(OPERATIONS):
            name, arguments = OPERATIONS[stage]
            block = {"type": "tool_use", "id": "synthetic_" + str(stage), "name": "mcp__discard-memory__" + name, "input": {}}
            events.append({"type": "content_block_start", "index": 0, "content_block": block})
            events.append({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": json.dumps(arguments)}})
        else:
            events.append({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": "Synthetic check complete."}})
        events.extend([
            {"type": "content_block_stop", "index": 0},
            {"type": "message_delta", "delta": {"stop_reason": "tool_use" if stage < len(OPERATIONS) else "end_turn", "stop_sequence": None}, "usage": {"output_tokens": 1}},
            {"type": "message_stop"},
        ])
        return events

    def verify(self, timeout=30):
        assert self.finished.wait(timeout), "Provider did not complete the synthetic tool sequence"
        assert self.receipts == OPERATIONS, "Provider did not execute the exact private save/read/shared proposal sequence"
        for index, expected in enumerate(EXPECTED_RESULTS):
            assert expected in self.results.get("synthetic_" + str(index), ""), "Synthetic tool result did not reach the model"
        assert self.private == {"items/synthetic.md": PRIVATE_TEXT}
        assert not self.shared, "Shared memory changed before approval"
        assert self.proposals == [{"scope": "global", "text": SHARED_TEXT, "status": "pending"}], "Shared proposal did not remain pending"
        assert not self.unexpected_mcp, "Disabled inherited MCP server received a request"
        if self.provider == "claude":
            allowed = {"mcp__discard-memory__" + name for name in TOOLS}
        else:
            allowed = {"mcp__discard_memory." + name for name in TOOLS} | {"functions.list_mcp_resources", "functions.list_mcp_resource_templates", "functions.read_mcp_resource"}
        assert self.tool_surfaces and all(surface == allowed for surface in self.tool_surfaces), "Model-facing tool allowlist changed"
        print("MODEL_TOOL_SURFACE", json.dumps(sorted(allowed)))
        print("MEMORY_EXECUTION", json.dumps({"private_save": "saved", "private_read": "returned", "shared_save": "pending", "shared_memory_changed": False}))


class ProbeProcess:
    """Drain provider output without retaining prompt/event bodies."""
    def __init__(self, process, provider):
        import queue
        self.process = process
        self.provider = provider
        self.responses = queue.Queue()
        self.requests = []
        self.lock = threading.Lock()
        threading.Thread(target=self.read, daemon=True).start()

    def send(self, value):
        with self.lock:
            self.process.stdin.write(json.dumps(value) + "\n")
            self.process.stdin.flush()

    def read(self):
        for line in self.process.stdout:
            try:
                value = json.loads(line)
            except ValueError:
                continue
            if self.provider == "claude":
                if value.get("type") == "control_request":
                    request = value.get("request", {})
                    self.requests.append(request.get("tool_name", request.get("subtype")))
                    self.send({"type": "control_response", "response": {"subtype": "success", "request_id": value["request_id"], "response": {"behavior": "deny", "message": "Synthetic probe grants no additional authority"}}})
            elif value.get("method") and value.get("id") is not None:
                self.requests.append(value["method"])
                self.send({"id": value["id"], "error": {"code": -32601, "message": "Synthetic probe grants no additional authority"}})
            elif value.get("method") is None and value.get("id") is not None:
                self.responses.put(value)

    def response(self, number):
        while True:
            value = self.responses.get(timeout=20)
            if value.get("id") == number:
                assert "error" not in value, "Provider rejected the synthetic probe request"
                return value["result"]
