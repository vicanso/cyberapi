#!/usr/bin/env python3
"""Seed an isolated temporary Harbor project in light mode for the demo video."""

from __future__ import annotations

import argparse
import json
import sqlite3
import tempfile
from pathlib import Path

HARBOR_COLL_ID = "01DEMOHARBOR00000000000001"
FOLDER_CATALOG_ID = "01DEMOFOLD0000000000000001"
FOLDER_LIVE_ID = "01DEMOFOLD0000000000000002"
FOLDER_SHOP_ID = "01DEMOFOLD0000000000000003"

ENV_LOCAL_ID = "01DEMOENV00000000000000001"
ENV_STAGING_ID = "01DEMOENV00000000000000002"

API_LIST_PRODUCTS = "01DEMOAPI00000000000000001"
API_CREATE_ORDER = "01DEMOAPI00000000000000002"
API_ORDER_EVENTS = "01DEMOAPI00000000000000003"
API_SHOP_GRAPHQL = "01DEMOAPI00000000000000004"

BASE_TIME = "2026-11-26T10:00:00Z"

SCHEMA_SQL = [
    """CREATE TABLE IF NOT EXISTS api_collections (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        name TEXT DEFAULT '',
        description TEXT DEFAULT '',
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT '',
        revision INTEGER NOT NULL DEFAULT 0
    )""",
    """CREATE TABLE IF NOT EXISTS api_folders (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        collection TEXT NOT NULL check (collection != ''),
        children TEXT DEFAULT '',
        name TEXT DEFAULT '',
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT ''
    )""",
    """CREATE TABLE IF NOT EXISTS api_settings (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        collection TEXT NOT NULL check (collection != ''),
        name TEXT DEFAULT '',
        category TEXT DEFAULT '',
        setting TEXT DEFAULT '',
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT ''
    )""",
    """CREATE TABLE IF NOT EXISTS variables (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        category TEXT NOT NULL check (category != ''),
        collection TEXT NOT NULL check (collection != ''),
        name TEXT DEFAULT '',
        value TEXT DEFAULT '',
        enabled TEXT DEFAULT '',
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT ''
    )""",
    """CREATE TABLE IF NOT EXISTS versions (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        version TEXT NOT NULL check (version != ''),
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT ''
    )""",
    """CREATE TABLE IF NOT EXISTS cyberapi_meta (
        key TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL DEFAULT ''
    )""",
    """CREATE TABLE IF NOT EXISTS top_orders (
        collection TEXT PRIMARY KEY NOT NULL,
        items TEXT NOT NULL DEFAULT ''
    )""",
    """CREATE TABLE IF NOT EXISTS prefs (
        scope TEXT PRIMARY KEY NOT NULL,
        doc TEXT NOT NULL DEFAULT '{}'
    )""",
    "CREATE INDEX IF NOT EXISTS api_folders_collection ON api_folders (collection)",
    "CREATE INDEX IF NOT EXISTS api_settings_collection ON api_settings (collection)",
    "CREATE INDEX IF NOT EXISTS variables_collection ON variables (collection, category)",
    """CREATE TABLE IF NOT EXISTS responses (
        id TEXT NOT NULL,
        collection TEXT NOT NULL,
        api TEXT NOT NULL,
        doc TEXT NOT NULL,
        created_at TEXT NOT NULL
    )""",
    "CREATE INDEX IF NOT EXISTS responses_api ON responses (collection, api)",
    """CREATE TABLE IF NOT EXISTS cookies (
        key TEXT PRIMARY KEY NOT NULL,
        doc TEXT NOT NULL
    )""",
    """CREATE TABLE IF NOT EXISTS server_copies (
        collection TEXT PRIMARY KEY NOT NULL,
        account TEXT NOT NULL,
        base INTEGER,
        unsynced INTEGER NOT NULL DEFAULT 0
    )""",
    """CREATE TABLE IF NOT EXISTS folder_settings (
        folder TEXT PRIMARY KEY NOT NULL,
        collection TEXT NOT NULL,
        settings TEXT NOT NULL
    )""",
    "CREATE INDEX IF NOT EXISTS folder_settings_collection ON folder_settings (collection)",
    """CREATE TABLE IF NOT EXISTS project_settings (
        collection TEXT PRIMARY KEY NOT NULL,
        settings TEXT NOT NULL
    )""",
    """CREATE TABLE IF NOT EXISTS environments (
        id TEXT PRIMARY KEY NOT NULL,
        collection TEXT NOT NULL,
        name TEXT NOT NULL,
        position INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    )""",
    "CREATE INDEX IF NOT EXISTS environments_collection ON environments (collection)",
    """CREATE TABLE IF NOT EXISTS variable_environments (
        variable TEXT PRIMARY KEY NOT NULL,
        collection TEXT NOT NULL,
        environment TEXT NOT NULL
    )""",
    "CREATE INDEX IF NOT EXISTS variable_environments_collection ON variable_environments (collection)",
]


def assert_isolated_dir(raw: str) -> Path:
    resolved = Path(raw).expanduser().resolve()
    allowed_roots = {
        Path("/tmp").resolve(),
        Path("/private/tmp").resolve(),
        Path(tempfile.gettempdir()).resolve(),
    }
    for root in allowed_roots:
        if resolved != root and root in resolved.parents:
            return resolved
    raise SystemExit(
        f"refusing non-temporary --data-dir {resolved}; must be a subdirectory under /tmp or {tempfile.gettempdir()}"
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data-dir", required=True)
    parser.add_argument("--base-url", default="http://127.0.0.1:17391")
    parser.add_argument("--db-name", default="cyberapi-dev.db")
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=720)
    args = parser.parse_args()

    data_dir = assert_isolated_dir(args.data_dir)
    data_dir.mkdir(parents=True, exist_ok=True)
    db_path = data_dir / args.db_name
    if db_path.exists():
        db_path.unlink()

    base_url = args.base_url.rstrip("/")
    conn = sqlite3.connect(db_path)
    try:
        for stmt in SCHEMA_SQL:
            conn.execute(stmt)
        conn.execute("INSERT OR REPLACE INTO cyberapi_meta (key, value) VALUES ('schema_version', '7')")

        conn.execute(
            "INSERT INTO api_collections (id, name, description, created_at, updated_at, revision) VALUES (?, ?, ?, ?, ?, ?)",
            (
                HARBOR_COLL_ID,
                "Harbor",
                "Storefront catalog, checkout orders, live event streams, and GraphQL queries.",
                BASE_TIME,
                BASE_TIME,
                1,
            ),
        )

        conn.executemany(
            "INSERT INTO environments (id, collection, name, position, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
            [
                (ENV_LOCAL_ID, HARBOR_COLL_ID, "local", 0, BASE_TIME, BASE_TIME),
                (ENV_STAGING_ID, HARBOR_COLL_ID, "staging", 1, BASE_TIME, BASE_TIME),
            ],
        )

        proj_settings = {
            "auth": {"kind": "bearer", "token": "harbor-live-secret-token-8821"},
            "source": {"url": f"{base_url}/openapi.json"},
        }
        conn.execute(
            "INSERT INTO project_settings (collection, settings) VALUES (?, ?)",
            (HARBOR_COLL_ID, json.dumps(proj_settings)),
        )

        variables = [
            ("01DEMOVAR00000000000000001", "env", HARBOR_COLL_ID, "host", base_url, "1", BASE_TIME, BASE_TIME),
            ("01DEMOVAR00000000000000002", "env", HARBOR_COLL_ID, "host", "https://staging.harbor.example", "1", BASE_TIME, BASE_TIME),
        ]
        conn.executemany(
            "INSERT INTO variables (id, category, collection, name, value, enabled, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            variables,
        )
        conn.executemany(
            "INSERT INTO variable_environments (variable, collection, environment) VALUES (?, ?, ?)",
            [
                ("01DEMOVAR00000000000000001", HARBOR_COLL_ID, ENV_LOCAL_ID),
                ("01DEMOVAR00000000000000002", HARBOR_COLL_ID, ENV_STAGING_ID),
            ],
        )

        folders = [
            (FOLDER_CATALOG_ID, HARBOR_COLL_ID, f"{API_LIST_PRODUCTS},{API_CREATE_ORDER}", "Catalog", BASE_TIME, BASE_TIME),
            (FOLDER_LIVE_ID, HARBOR_COLL_ID, API_ORDER_EVENTS, "Live", BASE_TIME, BASE_TIME),
            (FOLDER_SHOP_ID, HARBOR_COLL_ID, API_SHOP_GRAPHQL, "Shop", BASE_TIME, BASE_TIME),
        ]
        conn.executemany(
            "INSERT INTO api_folders (id, collection, children, name, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
            folders,
        )

        list_products_req = {
            "method": "GET",
            "uri": "{{host}}/v1/products",
            "contentType": "application/json",
            "body": "",
            "query": [
                {
                    "key": "inStock",
                    "value": "true",
                    "enabled": True,
                    "description": "In-stock items",
                    "schema_type": "boolean",
                    "required": False,
                }
            ],
            "headers": [],
            "assertions": [
                {"enabled": True, "subject": "status", "target": "", "op": "equals", "expected": "200"},
                {"enabled": True, "subject": "json", "target": "$.data[0].id", "op": "equals", "expected": "sku-1042"},
            ],
            "description": "Returns products in the Harbor catalog.",
        }

        create_order_req = {
            "method": "POST",
            "uri": "{{host}}/v1/orders",
            "contentType": "application/json",
            "body": json.dumps(
                {
                    "items": [
                        {"sku": "sku-1042", "qty": 1},
                        {"sku": "sku-1048", "qty": 1},
                    ],
                    "note": "Leave at front desk",
                },
                indent=4,
            ),
            "query": [],
            "headers": [],
            "assertions": [
                {"enabled": True, "subject": "status", "target": "", "op": "equals", "expected": "201"},
                {"enabled": True, "subject": "json", "target": "$.status", "op": "equals", "expected": "created"},
            ],
        }

        order_events_req = {
            "method": "GET",
            "uri": "{{host}}/v1/orders/events",
            "contentType": "application/json",
            "body": "",
            "query": [],
            "headers": [{"key": "Accept", "value": "text/event-stream", "enabled": True}],
        }

        shop_gql_req = {
            "method": "POST",
            "uri": "{{host}}/graphql",
            "contentType": "application/graphql",
            "body": json.dumps(
                {
                    "query": "query Shop($inStock: Boolean) {\n    shop {\n        name\n        products(inStock: $inStock) {\n            name\n            price\n        }\n    }\n}",
                    "variables": json.dumps({"inStock": True}, indent=4),
                    "operationName": "Shop",
                }
            ),
            "query": [],
            "headers": [],
        }

        apis = [
            (API_LIST_PRODUCTS, "List products", list_products_req),
            (API_CREATE_ORDER, "Create order", create_order_req),
            (API_ORDER_EVENTS, "Order events", order_events_req),
            (API_SHOP_GRAPHQL, "Shop", shop_gql_req),
        ]
        for api_id, name, req in apis:
            conn.execute(
                "INSERT INTO api_settings (id, collection, name, category, setting, created_at, updated_at) VALUES (?, ?, ?, 'http', ?, ?, ?)",
                (api_id, HARBOR_COLL_ID, name, json.dumps(req, ensure_ascii=False), BASE_TIME, BASE_TIME),
            )

        conn.execute(
            "INSERT INTO top_orders (collection, items) VALUES (?, ?)",
            (HARBOR_COLL_ID, f"{FOLDER_CATALOG_ID},{FOLDER_LIVE_ID},{FOLDER_SHOP_ID}"),
        )

        app_prefs = {
            "theme": "light",
            "language": "en",
            "text": {"uiSize": 14, "editorSize": 13, "wrap": True, "showTabs": True},
            "columns": {"tree": 225, "editorRatio": 0.48},
            "window": {"maximized": False, "width": args.width, "height": args.height},
            "monitors": {},
            "lastMonitor": "",
            "lastCollection": HARBOR_COLL_ID,
            "updates": {"autoCheck": False},
            "backups": {"auto": False},
        }
        workspace_prefs = {
            "expanded": [FOLDER_CATALOG_ID, FOLDER_LIVE_ID, FOLDER_SHOP_ID],
            "selected": API_LIST_PRODUCTS,
            "open": [API_LIST_PRODUCTS, API_CREATE_ORDER, API_ORDER_EVENTS, API_SHOP_GRAPHQL],
            "environment": ENV_LOCAL_ID,
        }
        conn.execute("INSERT INTO prefs (scope, doc) VALUES ('app', ?)", (json.dumps(app_prefs),))
        conn.execute(
            "INSERT INTO prefs (scope, doc) VALUES (?, ?)",
            (f"workspace/{HARBOR_COLL_ID}", json.dumps(workspace_prefs)),
        )
        conn.commit()
    finally:
        conn.close()


if __name__ == "__main__":
    main()
