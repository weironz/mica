"""Catch API environment allow-list drift between the two deployment forms."""

import json
import os
import pathlib
import subprocess


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
    print(f"API allow-lists match ({len(single)} keys); {len(PROBES)} settings pass through both forms")


if __name__ == "__main__":
    main()
