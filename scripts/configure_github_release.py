#!/usr/bin/env python3
"""Plan, apply, or verify Costguard's public GitHub release controls."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.error import HTTPError
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
API_ROOT = "https://api.github.com"
RELEASE_OWNER = "mattfaltyn"
RELEASE_OWNER_EMAIL = "faltyn.matthew@gmail.com"
RELEASE_TEAM = "release-owners"
RELEASE_ENVIRONMENT = "release"
RELEASE_VARIABLE = "RELEASE_SSH_ALLOWED_SIGNERS"
EXPECTED_KEY_FINGERPRINT = "SHA256:uiM1q8pDCkb7iW+6sNTblHdSYh4h0XUocIFIsUu8gGc"


class GitHubApiError(RuntimeError):
    def __init__(self, status: int, message: str):
        super().__init__(message)
        self.status = status


class GitHubClient:
    def __init__(self, token: str):
        self.token = token

    def request(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None = None,
    ) -> Any:
        data = None if payload is None else json.dumps(payload).encode("utf-8")
        request = Request(
            f"{API_ROOT}{path}",
            data=data,
            method=method,
            headers={
                "Accept": "application/vnd.github+json",
                "Authorization": f"Bearer {self.token}",
                "Content-Type": "application/json",
                "X-GitHub-Api-Version": "2022-11-28",
            },
        )
        try:
            with urlopen(request, timeout=30) as response:  # noqa: S310 - fixed origin
                if response.status == 204:
                    return None
                return json.load(response)
        except HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            raise GitHubApiError(exc.code, f"GitHub API {method} {path}: {body}") from exc

    def get_optional(self, path: str) -> Any | None:
        try:
            return self.request("GET", path)
        except GitHubApiError as exc:
            if exc.status == 404:
                return None
            raise

    def endpoint_enabled(self, path: str) -> bool:
        try:
            self.request("GET", path)
        except GitHubApiError as exc:
            if exc.status == 404:
                return False
            raise
        return True


@dataclass(frozen=True)
class Configuration:
    organization: str
    repository: str
    profile: str
    allowed_signers: str | None

    @property
    def full_name(self) -> str:
        return f"{self.organization}/{self.repository}"


@dataclass
class RepositoryState:
    public: bool
    default_branch: str
    owner_id: int
    team_id: int | None
    team_members: set[str]
    team_has_push: bool
    security: dict[str, bool]
    release_environment: dict[str, Any] | None
    release_variable: str | None
    rulesets: dict[str, dict[str, Any]]


def parse_repository(value: str) -> tuple[str, str]:
    parts = value.split("/")
    if len(parts) != 2 or not all(parts):
        raise SystemExit("--repository must use owner/name format")
    return parts[0], parts[1]


def load_allowed_signers(path: Path) -> str:
    lines = [line.strip() for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    if len(lines) != 1:
        raise SystemExit("allowed-signers file must contain exactly one non-empty line")
    fields = lines[0].split()
    if len(fields) < 3 or fields[0] != RELEASE_OWNER_EMAIL or fields[1] != "ssh-ed25519":
        raise SystemExit(
            f"allowed signer must be {RELEASE_OWNER_EMAIL} followed by an ssh-ed25519 key"
        )
    if "PRIVATE KEY" in lines[0]:
        raise SystemExit("allowed-signers file must not contain a private key")
    try:
        key_blob = base64.b64decode(fields[2], validate=True)
    except ValueError as exc:
        raise SystemExit("allowed signer contains invalid base64") from exc
    digest = base64.b64encode(hashlib.sha256(key_blob).digest()).decode("ascii").rstrip("=")
    fingerprint = f"SHA256:{digest}"
    if fingerprint != EXPECTED_KEY_FINGERPRINT:
        raise SystemExit(
            f"allowed signer fingerprint {fingerprint} != expected {EXPECTED_KEY_FINGERPRINT}"
        )
    return lines[0]


def required_checks(profile: str) -> tuple[str, ...]:
    if profile == "primary":
        return ("pr-gate", "scale", "costguard")
    return ("standard", "strict")


def ruleset_names(profile: str) -> tuple[str, str]:
    prefix = "costguard" if profile == "primary" else "costguard-consumer"
    return f"{prefix}-main-review-and-ci", f"{prefix}-main-integrity"


def desired_rulesets(config: Configuration, team_id: int) -> dict[str, dict[str, Any]]:
    review_name, integrity_name = ruleset_names(config.profile)
    conditions = {"ref_name": {"include": ["refs/heads/main"], "exclude": []}}
    bypass = [{"actor_id": team_id, "actor_type": "Team", "bypass_mode": "always"}]
    review = {
        "name": review_name,
        "target": "branch",
        "enforcement": "active",
        "bypass_actors": bypass,
        "conditions": conditions,
        "rules": [
            {"type": "required_linear_history"},
            {
                "type": "pull_request",
                "parameters": {
                    "allowed_merge_methods": ["merge", "squash", "rebase"],
                    "dismiss_stale_reviews_on_push": True,
                    "require_code_owner_review": False,
                    "require_last_push_approval": True,
                    "required_approving_review_count": 1,
                    "required_review_thread_resolution": True,
                },
            },
            {
                "type": "required_status_checks",
                "parameters": {
                    "do_not_enforce_on_create": False,
                    "required_status_checks": [
                        {"context": context} for context in required_checks(config.profile)
                    ],
                    "strict_required_status_checks_policy": True,
                },
            },
        ],
    }
    integrity = {
        "name": integrity_name,
        "target": "branch",
        "enforcement": "active",
        "bypass_actors": [],
        "conditions": conditions,
        "rules": [{"type": "deletion"}, {"type": "non_fast_forward"}],
    }
    return {review_name: review, integrity_name: integrity}


def normalized_ruleset(value: dict[str, Any]) -> dict[str, Any]:
    rules = []
    for rule in value.get("rules", []):
        item = {"type": rule.get("type")}
        if "parameters" in rule:
            parameters = dict(rule["parameters"])
            checks = parameters.get("required_status_checks")
            if isinstance(checks, list):
                parameters["required_status_checks"] = sorted(
                    ({"context": check.get("context")} for check in checks),
                    key=lambda check: str(check["context"]),
                )
            item["parameters"] = parameters
        rules.append(item)
    bypass = [
        {
            "actor_id": actor.get("actor_id"),
            "actor_type": actor.get("actor_type"),
            "bypass_mode": actor.get("bypass_mode"),
        }
        for actor in value.get("bypass_actors", [])
    ]
    return {
        "name": value.get("name"),
        "target": value.get("target"),
        "enforcement": value.get("enforcement"),
        "bypass_actors": sorted(bypass, key=lambda actor: json.dumps(actor, sort_keys=True)),
        "conditions": value.get("conditions"),
        "rules": sorted(rules, key=lambda rule: str(rule["type"])),
    }


def environment_matches(value: dict[str, Any] | None, owner_id: int) -> bool:
    if value is None:
        return False
    required_reviewers = next(
        (
            rule
            for rule in value.get("protection_rules", [])
            if rule.get("type") == "required_reviewers"
        ),
        None,
    )
    if required_reviewers is None:
        return False
    reviewer_ids = {
        item.get("reviewer", {}).get("id")
        for item in required_reviewers.get("reviewers", [])
        if item.get("type") == "User"
    }
    prevent_self_review = required_reviewers.get("prevent_self_review")
    return reviewer_ids == {owner_id} and prevent_self_review is False


def configuration_drift(state: RepositoryState, config: Configuration) -> list[str]:
    if not state.public:
        return [f"{config.full_name} is private; visibility changes are intentionally manual"]
    drift: list[str] = []
    if state.team_id is None:
        drift.append(f"create @{config.organization}/{RELEASE_TEAM} with Matt as sole member")
    if state.team_members != {RELEASE_OWNER}:
        drift.append(
            f"set @{config.organization}/{RELEASE_TEAM} members to [{RELEASE_OWNER}]"
        )
    if not state.team_has_push:
        drift.append(f"grant @{config.organization}/{RELEASE_TEAM} push access")
    for name, enabled in state.security.items():
        if not enabled:
            drift.append(f"enable {name.replace('_', ' ')}")
    if config.profile == "primary":
        if not environment_matches(state.release_environment, state.owner_id):
            drift.append("configure release environment for Matt-only self approval")
        if state.release_variable != config.allowed_signers:
            drift.append(f"set repository variable {RELEASE_VARIABLE}")
    if state.team_id is None:
        drift.append("create managed rulesets after the release-owners team exists")
    else:
        for name, expected in desired_rulesets(config, state.team_id).items():
            actual = state.rulesets.get(name)
            if actual is None or normalized_ruleset(actual) != normalized_ruleset(expected):
                drift.append(f"create or update ruleset {name}")
    return drift


def read_state(client: GitHubClient, config: Configuration) -> RepositoryState:
    repo_path = f"/repos/{config.full_name}"
    repo = client.request("GET", repo_path)
    owner = client.request("GET", f"/users/{RELEASE_OWNER}")
    if repo.get("visibility") != "public":
        return RepositoryState(
            public=False,
            default_branch=str(repo.get("default_branch", "main")),
            owner_id=int(owner["id"]),
            team_id=None,
            team_members=set(),
            team_has_push=False,
            security={},
            release_environment=None,
            release_variable=None,
            rulesets={},
        )
    teams = client.request("GET", f"/orgs/{config.organization}/teams?per_page=100")
    team = next((item for item in teams if item.get("slug") == RELEASE_TEAM), None)
    team_id = None if team is None else int(team["id"])
    members: set[str] = set()
    team_has_push = False
    if team is not None:
        members = {
            str(member["login"])
            for member in client.request(
                "GET", f"/orgs/{config.organization}/teams/{RELEASE_TEAM}/members?per_page=100"
            )
        }
        team_repo = client.get_optional(
            f"/orgs/{config.organization}/teams/{RELEASE_TEAM}/repos/{config.full_name}"
        )
        team_has_push = bool(team_repo and team_repo.get("permissions", {}).get("push"))

    security_analysis = repo.get("security_and_analysis") or {}
    security = {
        "advanced_security": security_analysis.get("advanced_security", {}).get("status")
        == "enabled",
        "secret_scanning": security_analysis.get("secret_scanning", {}).get("status")
        == "enabled",
        "secret_scanning_push_protection": security_analysis.get(
            "secret_scanning_push_protection", {}
        ).get("status")
        == "enabled",
        "private_vulnerability_reporting": bool(
            (client.get_optional(f"{repo_path}/private-vulnerability-reporting") or {}).get(
                "enabled"
            )
        ),
        "vulnerability_alerts": client.endpoint_enabled(f"{repo_path}/vulnerability-alerts"),
        "automated_security_fixes": bool(
            (client.get_optional(f"{repo_path}/automated-security-fixes") or {}).get("enabled")
        ),
    }
    environment = None
    variable = None
    if config.profile == "primary":
        environment = client.get_optional(f"{repo_path}/environments/{RELEASE_ENVIRONMENT}")
        variables = client.request("GET", f"{repo_path}/actions/variables?per_page=100")
        variable = next(
            (
                item.get("value")
                for item in variables.get("variables", [])
                if item.get("name") == RELEASE_VARIABLE
            ),
            None,
        )
    summaries = client.request("GET", f"{repo_path}/rulesets?per_page=100")
    managed_names = set(ruleset_names(config.profile))
    rulesets = {
        summary["name"]: client.request("GET", f"{repo_path}/rulesets/{summary['id']}")
        for summary in summaries
        if summary.get("name") in managed_names
    }
    return RepositoryState(
        public=repo.get("visibility") == "public",
        default_branch=str(repo.get("default_branch", "main")),
        owner_id=int(owner["id"]),
        team_id=team_id,
        team_members=members,
        team_has_push=team_has_push,
        security=security,
        release_environment=environment,
        release_variable=variable,
        rulesets=rulesets,
    )


def ensure_team(client: GitHubClient, config: Configuration) -> int:
    teams = client.request("GET", f"/orgs/{config.organization}/teams?per_page=100")
    team = next((item for item in teams if item.get("slug") == RELEASE_TEAM), None)
    if team is None:
        team = client.request(
            "POST",
            f"/orgs/{config.organization}/teams",
            {
                "name": RELEASE_TEAM,
                "description": "Matt-only Costguard release and direct-main bypass",
                "maintainers": [RELEASE_OWNER],
                "repo_names": [config.full_name],
                "permission": "push",
                "privacy": "closed",
            },
        )
    client.request(
        "PUT",
        f"/orgs/{config.organization}/teams/{RELEASE_TEAM}/memberships/{RELEASE_OWNER}",
        {"role": "maintainer"},
    )
    members = client.request(
        "GET", f"/orgs/{config.organization}/teams/{RELEASE_TEAM}/members?per_page=100"
    )
    for member in members:
        if member.get("login") != RELEASE_OWNER:
            client.request(
                "DELETE",
                f"/orgs/{config.organization}/teams/{RELEASE_TEAM}/memberships/"
                f"{member['login']}",
            )
    client.request(
        "PUT",
        f"/orgs/{config.organization}/teams/{RELEASE_TEAM}/repos/{config.full_name}",
        {"permission": "push"},
    )
    return int(team["id"])


def apply_configuration(client: GitHubClient, config: Configuration) -> None:
    repo_path = f"/repos/{config.full_name}"
    repo = client.request("GET", repo_path)
    if repo.get("visibility") != "public":
        raise SystemExit(
            f"{config.full_name} is private; make it public explicitly before applying controls"
        )
    team_id = ensure_team(client, config)
    client.request(
        "PATCH",
        repo_path,
        {
            "security_and_analysis": {
                "advanced_security": {"status": "enabled"},
                "secret_scanning": {"status": "enabled"},
                "secret_scanning_push_protection": {"status": "enabled"},
            }
        },
    )
    client.request("PUT", f"{repo_path}/private-vulnerability-reporting")
    client.request("PUT", f"{repo_path}/vulnerability-alerts")
    client.request("PUT", f"{repo_path}/automated-security-fixes")
    if config.profile == "primary":
        owner = client.request("GET", f"/users/{RELEASE_OWNER}")
        client.request(
            "PUT",
            f"{repo_path}/environments/{RELEASE_ENVIRONMENT}",
            {
                "wait_timer": 0,
                "prevent_self_review": False,
                "reviewers": [{"type": "User", "id": owner["id"]}],
                "deployment_branch_policy": None,
            },
        )
        variables = client.request("GET", f"{repo_path}/actions/variables?per_page=100")
        exists = any(
            item.get("name") == RELEASE_VARIABLE for item in variables.get("variables", [])
        )
        method = "PATCH" if exists else "POST"
        path = (
            f"{repo_path}/actions/variables/{RELEASE_VARIABLE}"
            if exists
            else f"{repo_path}/actions/variables"
        )
        client.request(
            method,
            path,
            {"name": RELEASE_VARIABLE, "value": config.allowed_signers},
        )
    summaries = client.request("GET", f"{repo_path}/rulesets?per_page=100")
    by_name = {item.get("name"): item for item in summaries}
    for name, payload in desired_rulesets(config, team_id).items():
        existing = by_name.get(name)
        if existing is None:
            client.request("POST", f"{repo_path}/rulesets", payload)
        else:
            client.request("PUT", f"{repo_path}/rulesets/{existing['id']}", payload)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--plan", action="store_true")
    mode.add_argument("--apply", action="store_true")
    mode.add_argument("--verify", action="store_true")
    parser.add_argument("--repository", default="hypertrial/costguard")
    parser.add_argument("--profile", choices=["primary", "consumer"], default="primary")
    parser.add_argument(
        "--allowed-signers-file",
        type=Path,
        default=ROOT / ".github/release_allowed_signers",
    )
    args = parser.parse_args()
    organization, repository = parse_repository(args.repository)
    allowed_signers = (
        load_allowed_signers(args.allowed_signers_file) if args.profile == "primary" else None
    )
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if not token:
        raise SystemExit("GH_TOKEN or GITHUB_TOKEN is required")
    client = GitHubClient(token)
    config = Configuration(organization, repository, args.profile, allowed_signers)
    state = read_state(client, config)
    drift = configuration_drift(state, config)
    if args.plan:
        if drift:
            for item in drift:
                print(f"PLAN {item}")
        else:
            print(f"{config.full_name} already matches the requested controls")
        return 0 if state.public else 1
    if args.verify:
        if drift:
            for item in drift:
                print(f"FAIL {item}", file=sys.stderr)
            return 1
        print(f"verified GitHub release controls for {config.full_name}")
        return 0
    if not state.public:
        raise SystemExit(
            f"{config.full_name} is private; make it public explicitly before applying controls"
        )
    apply_configuration(client, config)
    drift = configuration_drift(read_state(client, config), config)
    if drift:
        for item in drift:
            print(f"FAIL {item}", file=sys.stderr)
        return 1
    print(f"applied and verified GitHub release controls for {config.full_name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
