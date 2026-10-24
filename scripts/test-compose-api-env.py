"""Catch API allow-list drift and loss of shared-ingress working-set protection."""

import configparser
import json
import os
import pathlib
import subprocess
from urllib.parse import urlsplit


ROOT = pathlib.Path(__file__).resolve().parents[1]
PROBES = {
    "MICA_REGISTRATION_ENABLED": "true",
    "MICA_ADMIN_EMAIL": "admin@example.test",
    "MICA_WORKSPACE_QUOTA_BYTES": "123456",
    "MICA_WS_MIN_PROTOCOL": "5",
    "MICA_CATCH_UP_LIMIT": "123",
    "MICA_STREAM_KEEP_MARGIN": "45",
    "MICA_STREAM_PRUNE_EVERY": "6",
    "CORS_ALLOWED_ORIGINS": "https://app.example.test",
    "ACCESS_TOKEN_TTL_SECONDS": "321",
    "REFRESH_TOKEN_TTL_SECONDS": "654",
    "DATABASE_MAX_CONNECTIONS": "7",
    "MICA_APP_BASE_URL": "https://custom.example.test",
    "MICA_MAIL_BACKEND": "directmail",
    "MICA_MAIL_FROM": "mail@example.test",
    "MICA_MAIL_FROM_NAME": "Mica Test",
    "MICA_MAIL_ACCESS_KEY_ID": "test-access-key",
    "MICA_MAIL_SECRET_ACCESS_KEY": "test-secret-key",
    "MICA_MAIL_REGION": "cn-shanghai",
    "MICA_MAIL_ENDPOINT": "https://dm.example.test",
}


def api_environment(compose: str, env: dict[str, str]) -> dict[str, str]:
    result = subprocess.run(
        ["docker", "compose", "-f", str(ROOT / "deploy" / compose), "config", "--format", "json"],
        cwd=ROOT,
        env=env,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)["services"]["api"]["environment"]


def check_traefik_working_set(env: dict[str, str]) -> None:
    probe_env = env | {
        "TRAEFIK_IMAGE_TAG": "traefik:v3.6.10",
        "TRAEFIK_LOG_LEVEL": "WARN",
        "TRAEFIK_ACME_EMAIL": "admin@example.test",
        "TRAEFIK_HOSTNAME": "traefik.example.test",
        "TRAEFIK_BASIC_AUTH": "test:unused",
    }
    result = subprocess.run(
        ["docker", "compose", "-f", str(ROOT / "deploy/traefik/docker-compose.yml"),
         "config", "--format", "json"],
        cwd=ROOT, env=probe_env, check=True, capture_output=True, text=True,
    )
    service = json.loads(result.stdout)["services"]["traefik"]
    expected = {
        "mem_limit": 256 * 1024 * 1024,
        "mem_reservation": 128 * 1024 * 1024,
        "memswap_limit": -1,
        "cgroup_parent": "traefik.slice",
    }
    for key, value in expected.items():
        actual = service.get(key)
        # Compose versions serialize byte counts as either numbers or strings.
        if isinstance(value, int) and actual is not None:
            actual = int(actual)
        if actual != value:
            raise AssertionError(f"Traefik {key}: expected {value!r}, got {actual!r}")
    unit = configparser.ConfigParser()
    unit.read(ROOT / "deploy/traefik" / service["cgroup_parent"], encoding="utf-8")
    if unit["Slice"].get("MemoryLow") != "128M":
        raise AssertionError("Traefik root protection must match its 128 MiB leaf reservation")
    if unit["Slice"].get("MemoryMin", "0") != "0":
        raise AssertionError("Traefik must use soft protection, not MemoryMin")
    if unit["Slice"].get("MemoryAccounting", "").lower() != "yes":
        raise AssertionError("Traefik slice must account for memory")
    if unit["Install"].get("WantedBy") != "slices.target":
        raise AssertionError("Traefik slice must remain enabled across boots")
    print("Traefik: 256 MiB cap, 128 MiB root/leaf protection, unlimited swap")


def main() -> None:
    env = os.environ.copy()
    env.update(
        MICA_VERSION="v0.0.0",
        SERVER_IP="127.0.0.1",
        DOMAIN="example.test",
        S3_DOMAIN="s3.example.test",
        **PROBES,
    )
    traefik = api_environment("docker-compose.yml", env)
    single = api_environment("docker-compose.single.yml", env)
    if traefik.keys() != single.keys():
        raise AssertionError(
            f"API allow-lists differ: Traefik only={sorted(traefik.keys() - single.keys())}, "
            f"single only={sorted(single.keys() - traefik.keys())}"
        )
    for name, expected in PROBES.items():
        for form, actual in (("Traefik", traefik), ("single", single)):
            if actual[name] != expected:
                raise AssertionError(f"{form} did not pass through {name}")
    default_env = env.copy()
    del default_env["MICA_APP_BASE_URL"]
    if api_environment("docker-compose.yml", default_env)["MICA_APP_BASE_URL"] != "https://example.test":
        raise AssertionError("Traefik reset-link default must use its HTTPS domain")
    if api_environment("docker-compose.single.yml", default_env)["MICA_APP_BASE_URL"] != "http://127.0.0.1":
        raise AssertionError("quickstart reset-link default must use its HTTP address")
    check_traefik_working_set(env)
    for compose in ("docker-compose.yml", "docker-compose.single.yml"):
        default_database_env = env.copy()
        default_database_env["MICA_DATABASE_HOST"] = ""
        default_url = urlsplit(api_environment(compose, default_database_env)["DATABASE_URL"])
        if default_url.hostname != "postgres":
            raise AssertionError(f"{compose}: default database alias changed")
        pinned_url = urlsplit(api_environment(
            compose, default_database_env | {"MICA_DATABASE_HOST": "192.0.2.123"}
        )["DATABASE_URL"])
        if pinned_url.hostname != "192.0.2.123":
            raise AssertionError(f"{compose}: database host override was ignored")
        if (pinned_url.username, pinned_url.password, pinned_url.port, pinned_url.path) != (
            default_url.username, default_url.password, default_url.port, default_url.path
        ):
            raise AssertionError(f"{compose}: host override changed credentials or database")
    print("Database host: default alias and explicit override preserve credentials in both stacks")
    print(f"API allow-lists match ({len(single)} keys); {len(PROBES)} settings pass through both forms")


if __name__ == "__main__":
    main()
