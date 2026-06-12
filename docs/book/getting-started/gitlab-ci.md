# GitLab CI integration

Run Costguard in GitLab pipelines using the release binary or a Rust build stage.

## Example job

```yaml
costguard:
  stage: test
  image: ubuntu:24.04
  variables:
    COSTGUARD_VERSION: "v1.0.0"
  before_script:
    - apt-get update && apt-get install -y curl ca-certificates
    - curl -fsSL "https://github.com/hypertrial/costguard/releases/download/${COSTGUARD_VERSION}/costguard-x86_64-unknown-linux-gnu.tar.gz" -o /tmp/costguard.tgz
    - curl -fsSL "https://github.com/hypertrial/costguard/releases/download/${COSTGUARD_VERSION}/costguard-x86_64-unknown-linux-gnu.tar.gz.sha256" -o /tmp/costguard.tgz.sha256
    - echo "$(cat /tmp/costguard.tgz.sha256 | awk '{print $1}')  /tmp/costguard.tgz" | sha256sum -c -
    - tar -xzf /tmp/costguard.tgz -C /usr/local/bin
  script:
    - dbt compile --target dev   # optional; or use use-existing-manifest
    - costguard pr --base "${CI_MERGE_REQUEST_DIFF_BASE_SHA:-origin/main}" \
        --warehouse trino \
        --manifest target/manifest.json \
        --baseline costguard-baseline.json \
        --fail-on high \
        --format sarif > costguard.sarif
  artifacts:
    reports:
      sast: costguard.sarif
    when: always
```

## Notes

- Use `--format sarif` for GitLab SAST report ingestion (`reports: sast`).
- Use `--format json` for custom gates or merge-request comments via a small script.
- Pin the release version and verify SHA-256 checksums before extracting the binary.
- For monorepos, compile each dbt subproject and merge manifests before scan (see `scripts/dbt_compile_for_costguard.py`).
