import http.server, socketserver

class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(n).decode()
        print("[zap-ipc-received]", body, flush=True)
        self.send_response(200); self.send_header('Content-Type','text/plain'); self.end_headers()
        self.wfile.write(b"ack")
    def log_message(self, *a): pass

class S(socketserver.TCPServer):
    allow_reuse_address = True

with S(("127.0.0.1", 9911), H) as s:
    print("server up", flush=True)
    s.serve_forever()
