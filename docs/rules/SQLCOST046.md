# SQLCOST046: Manifest checksum mismatch

**Severity:** low

Reports when a changed dbt model file's sha256 checksum does not match the loaded manifest node. Costguard cannot verify that compiled SQL and lineage metadata represent the PR head.

## When it fires

- PR mode with a loaded manifest that includes sha256 checksums for changed models.
- The workspace model SQL content hash differs from the manifest checksum.

## Fix

Re-run dbt compile so the manifest reflects current models, then scan again:

```bash
dbt compile && costguard pr --base origin/main
```

## Note

Checksum comparison is preferred over file modification times when dbt wrote checksum metadata. A future `require_manifest_integrity` analysis flag will fail closed on mismatches; today this is advisory only.
