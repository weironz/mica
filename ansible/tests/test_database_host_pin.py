"""Exercise the real deploy preflight tasks locally; requires the Ansible toolchain."""

import pathlib
import shutil
import subprocess
import sys
import tempfile

import yaml


ROOT = pathlib.Path(__file__).resolve().parents[2]
TASK_NAMES = {
    "Reduce the database inspection to address metadata",
    "Require one running database address for automatic pinning",
    "Resolve the automatic database pin before rendering",
    "Refuse a stale database address before changing deployment files",
}


def main():
    play = yaml.safe_load((ROOT / "ansible/deploy.yml").read_text(encoding="utf-8"))[0]
    tasks = [task for task in play["tasks"] if task.get("name") in TASK_NAMES]
    assert len(tasks) == len(TASK_NAMES), "Database preflight tasks moved; update this test"
    ansible = shutil.which("ansible-playbook")
    assert ansible, "Run inside the Ansible controller environment"
    cases = [
        ("matching address", "192.0.2.10", True, "192.0.2.10", True),
        ("stale address", "192.0.2.11", True, "192.0.2.10", False),
        ("stopped database", "192.0.2.10", False, "192.0.2.10", False),
        ("no active address", "192.0.2.10", True, "", False),
        ("automatic current address", "auto", True, "192.0.2.10", True),
        ("automatic reassigned address", "auto", True, "192.0.2.20", True),
        ("automatic stopped database", "auto", False, "192.0.2.10", False),
        ("automatic empty address", "auto", True, "", False),
        ("automatic ambiguous address", "auto", True, ["192.0.2.10", "192.0.2.20"], False),
    ]
    with tempfile.TemporaryDirectory(prefix="mica-database-pin-") as directory:
        target = pathlib.Path(directory) / "check.yml"
        for name, configured, running, actual, accepted in cases:
            addresses = actual if isinstance(actual, list) else [actual]
            expected = addresses[0] if configured == "auto" else configured
            fixture = [{
                "name": name, "hosts": "localhost", "connection": "local",
                "gather_facts": False,
                "vars": {
                    "ansible_python_interpreter": sys.executable,
                    "mica_env": {"MICA_DATABASE_HOST": configured},
                    "pinned_postgres_raw": {"container": {
                        "State": {"Running": running},
                        "NetworkSettings": {"Networks": {
                            f"mica_network_{index}": {"IPAddress": address}
                            for index, address in enumerate(addresses)
                        }},
                    }},
                },
                "tasks": tasks + [{
                    "name": "Accepted environments contain the verified address, never auto",
                    "ansible.builtin.assert": {"that": ["mica_env.MICA_DATABASE_HOST == expected_address"]},
                    "vars": {"expected_address": expected},
                }],
            }]
            target.write_text(yaml.safe_dump(fixture, allow_unicode=True), encoding="utf-8")
            result = subprocess.run(
                [ansible, "-i", "localhost,", str(target)],
                capture_output=True, text=True, timeout=60,
            )
            assert (result.returncode == 0) == accepted, result.stdout + result.stderr
            print(f"{name}: {'accepted' if accepted else 'rejected'}")


if __name__ == "__main__":
    main()
