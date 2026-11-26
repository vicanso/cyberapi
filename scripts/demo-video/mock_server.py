#!/usr/bin/env python3
"""Local HTTP / SSE / GraphQL / OpenAPI mock server for the CyberAPI demo video."""

from __future__ import annotations

import http.server
import json
import sys
import time
from urllib.parse import urlparse


PRODUCTS = [
    {"id": "sku-1042", "name": "Harbor mug", "price": 18, "stock": 42},
    {"id": "sku-1048", "name": "Cedar tray", "price": 36, "stock": 11},
    {"id": "sku-1102", "name": "Linen tote", "price": 24, "stock": 7},
]


def build_openapi(base_url: str) -> dict:
    return {
        "openapi": "3.0.3",
        "info": {"title": "host", "version": "1.2.0"},
        "servers": [{"url": base_url}],
        "paths": {
            "/v1/products": {
                "get": {
                    "tags": ["Catalog"],
                    "summary": "List products",
                    "description": "Returns products in the Harbor catalog.",
                    "parameters": [
                        {
                            "name": "inStock",
                            "in": "query",
                            "required": False,
                            "description": "In-stock items",
                            "schema": {"type": "boolean"},
                        }
                    ],
                    "responses": {"200": {"description": "OK"}},
                }
            },
            "/v1/orders": {
                "post": {
                    "tags": ["Catalog"],
                    "summary": "Create order",
                    "requestBody": {
                        "required": True,
                        "content": {
                            "application/json": {
                                "example": {
                                    "items": [
                                        {"sku": "sku-1042", "qty": 1},
                                        {"sku": "sku-1048", "qty": 1},
                                    ],
                                    "note": "Leave at front desk",
                                }
                            }
                        },
                    },
                    "responses": {"201": {"description": "Created"}},
                }
            },
            "/v1/products/{id}": {
                "get": {
                    "tags": ["Catalog"],
                    "summary": "Get a product",
                    "description": "Fetch details and inventory for a single product SKU.",
                    "parameters": [
                        {
                            "name": "id",
                            "in": "path",
                            "required": True,
                            "description": "Product SKU",
                            "schema": {"type": "string"},
                            "example": "sku-1042",
                        }
                    ],
                    "responses": {"200": {"description": "OK"}},
                }
            },
        },
    }


class DemoHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "HarborMock/1.0"

    def log_message(self, format: str, *args) -> None:  # noqa: A002
        return

    def send_json(self, status: int, payload: object, extra_headers: dict[str, str] | None = None) -> None:
        raw = json.dumps(payload, ensure_ascii=False, indent=2).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("X-Request-Id", "req-harbor-01J9")
        self.send_header("X-RateLimit-Remaining", "59")
        if extra_headers:
            for k, v in extra_headers.items():
                self.send_header(k, v)
        self.end_headers()
        self.wfile.write(raw)

    def read_body(self) -> bytes:
        length = int(self.headers.get("Content-Length", "0") or "0")
        return self.rfile.read(length) if length > 0 else b""

    def do_GET(self) -> None:  # noqa: N802
        parsed = urlparse(self.path)
        path = parsed.path

        if path == "/v1/products":
            time.sleep(0.035)
            self.send_json(200, {"data": PRODUCTS})
            return

        if path.startswith("/v1/products/"):
            sku = path.removeprefix("/v1/products/") or "sku-1042"
            time.sleep(0.025)
            found = next((p for p in PRODUCTS if p["id"] == sku), PRODUCTS[0])
            self.send_json(
                200,
                {
                    "id": found["id"],
                    "name": found["name"],
                    "price": found["price"],
                    "stock": found["stock"],
                    "currency": "USD",
                    "inStock": found["stock"] > 0,
                },
            )
            return

        if path == "/v1/orders/events":
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream; charset=utf-8")
            self.send_header("Cache-Control", "no-cache")
            self.send_header("Connection", "close")
            self.end_headers()
            events = [
                ("order.created", {"orderId": "ord-2026-0842", "status": "created", "total": 54}),
                ("payment.authorized", {"orderId": "ord-2026-0842", "provider": "stripe", "amount": 54}),
                ("inventory.reserved", {"orderId": "ord-2026-0842", "skus": ["sku-1042", "sku-1048"]}),
                ("warehouse.packed", {"orderId": "ord-2026-0842", "hub": "west-01", "weightKg": 1.2}),
                ("shipment.dispatched", {"orderId": "ord-2026-0842", "carrier": "UPS", "tracking": "1Z999AA10123"}),
                ("shipment.in_transit", {"orderId": "ord-2026-0842", "facility": "Oakland Hub"}),
                ("delivery.scheduled", {"orderId": "ord-2026-0842", "eta": "2026-11-28"}),
            ]
            try:
                for idx, (ev_name, data) in enumerate(events, start=1):
                    frame = f"id: {idx}\nevent: {ev_name}\ndata: {json.dumps(data)}\n\n".encode("utf-8")
                    self.wfile.write(frame)
                    self.wfile.flush()
                    time.sleep(0.55)
            except (BrokenPipeError, ConnectionResetError, OSError):
                return
            return

        if path == "/openapi.json":
            host = self.headers.get("Host") or f"127.0.0.1:{self.server.server_port}"
            self.send_json(200, build_openapi(f"http://{host}"))
            return

        self.send_json(404, {"error": "not found", "path": path})

    def do_POST(self) -> None:  # noqa: N802
        parsed = urlparse(self.path)
        _ = self.read_body()

        if parsed.path == "/v1/orders":
            time.sleep(0.045)
            self.send_json(
                201,
                {
                    "id": "ord-2026-0842",
                    "status": "created",
                    "currency": "USD",
                    "total": 54,
                    "items": [
                        {"sku": "sku-1042", "qty": 1},
                        {"sku": "sku-1048", "qty": 1},
                    ],
                    "createdAt": "2026-11-26T10:15:00Z",
                },
            )
            return

        if parsed.path == "/graphql":
            time.sleep(0.03)
            self.send_json(
                200,
                {
                    "data": {
                        "shop": {
                            "name": "Harbor",
                            "products": [
                                {"name": "Harbor mug", "price": 18},
                                {"name": "Cedar tray", "price": 36},
                            ],
                        }
                    }
                },
            )
            return

        self.send_json(404, {"error": "not found"})


def main() -> None:
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 17391
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), DemoHandler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
