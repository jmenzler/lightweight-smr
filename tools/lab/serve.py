#!/usr/bin/env python3
"""Lab dev server: http.server + Cache-Control: no-store, so ES modules and
wasm always reload fresh (Chrome caches module imports across deploys)."""
import http.server
import sys


class NoStoreHandler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8791
    http.server.ThreadingHTTPServer(("", port), NoStoreHandler).serve_forever()
