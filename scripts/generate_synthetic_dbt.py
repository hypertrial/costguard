#!/usr/bin/env python3
"""Generate a synthetic dbt-style project for opt-in Costguard scale tests."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--models", type=int, default=1000)
    args = parser.parse_args()

    root = args.output
    models_dir = root / "models" / "generated"
    target_dir = root / "target"
    models_dir.mkdir(parents=True, exist_ok=True)
    target_dir.mkdir(parents=True, exist_ok=True)

    nodes: dict[str, object] = {}
    for idx in range(args.models):
        name = f"model_{idx:04d}"
        path = Path("models") / "generated" / f"{name}.sql"
        previous = f"model_{idx - 1:04d}" if idx else None
        if previous:
            sql = (
                "{{ config(materialized='incremental') }}\n\n"
                "select\n"
                "  id,\n"
                "  json_extract_scalar(payload, '$.session_id') as session_id,\n"
                "  row_number() over () as rn\n"
                f"from {{{{ ref('{previous}') }}}}\n"
            )
            refs = [[previous]]
            depends_on = [f"model.synthetic.{previous}"]
        else:
            sql = (
                "select\n"
                "  id,\n"
                "  payload\n"
                "from {{ source('raw', 'events') }}\n"
            )
            refs = []
            depends_on = ["source.synthetic.raw.events"]
        (root / path).write_text(sql, encoding="utf-8")
        nodes[f"model.synthetic.{name}"] = {
            "resource_type": "model",
            "name": name,
            "original_file_path": path.as_posix(),
            "config": {"materialized": "incremental" if previous else "view"},
            "refs": refs,
            "sources": [["raw", "events"]] if not previous else [],
            "depends_on": {"nodes": depends_on},
            "columns": {"id": {}},
            "tags": ["synthetic"],
        }

    manifest = {
        "nodes": nodes,
        "sources": {
            "source.synthetic.raw.events": {
                "source_name": "raw",
                "name": "events",
            }
        },
    }
    (target_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2),
        encoding="utf-8",
    )
    (root / "schema.yml").write_text(
        "version: 2\nsources:\n  - name: raw\n    tables:\n      - name: events\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
