# Jenkins integration

Run Costguard from a Jenkins pipeline using a pinned release binary.

## Declarative pipeline example

```groovy
pipeline {
  agent any
  environment {
    COSTGUARD_VERSION = 'v2.0.0-rc.1'
  }
  stages {
    stage('Install Costguard') {
      steps {
        sh '''
          curl -fsSL "https://github.com/hypertrial/costguard/releases/download/${COSTGUARD_VERSION}/costguard-x86_64-unknown-linux-gnu.tar.gz" -o costguard.tgz
          curl -fsSL "https://github.com/hypertrial/costguard/releases/download/${COSTGUARD_VERSION}/costguard-x86_64-unknown-linux-gnu.tar.gz.sha256" -o costguard.tgz.sha256
          echo "$(awk '{print $1}' costguard.tgz.sha256)  costguard.tgz" | sha256sum -c -
          tar -xzf costguard.tgz -C /usr/local/bin
        '''
      }
    }
    stage('dbt compile') {
      steps {
        sh 'dbt compile --target dev'
      }
    }
    stage('Costguard') {
      steps {
        sh '''
          costguard pr --base origin/main \
            --warehouse snowflake \
            --manifest target/manifest.json \
            --baseline costguard-baseline.json \
            --fail-on high \
            --format sarif > costguard.sarif
          # Optional: add --cost --fail-on-cost-delta 500 when costguard.toml [cost] is configured
        '''
      }
    }
  }
  post {
    always {
      archiveArtifacts artifacts: 'costguard.sarif', fingerprint: true
    }
  }
}
```

## SARIF in Jenkins

- Archive `costguard.sarif` as a build artifact.
- Optionally publish with a SARIF plugin if your controller supports it.
- Use `--format json` and parse `metrics.new_findings` for simple pass/fail gates.
- Optional cost estimates: configure `[cost]` in `costguard.toml` or pass `--cost` for advisory prioritization. See [Cost estimates](../reference/cost-estimates.md).

## Baseline workflow

1. Generate `costguard-baseline.json` once on `main` with `--write-baseline`.
2. Store in SCM or a shared config repository.
3. Pass `--baseline` on every PR build so only **new** findings fail the job.
