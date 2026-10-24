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
    ]
    with tempfile.TemporaryDirectory(prefix="mica-database-pin-") as directory:
        target = pathlib.Path(directory) / "check.yml"
        for name, configured, running, actual, accepted in cases:
            fixture = [{
                "name": name, "hosts": "localhost", "connection": "local",
                "gather_facts": False,
                "vars": {
                    "ansible_python_interpreter": sys.executable,
                    "mica_env": {"MICA_DATABASE_HOST": configured},
                    "pinned_postgres_raw": {"container": {
                        "State": {"Running": running},
                        "NetworkSettings": {"Networks": {"mica_default": {"IPAddress": actual}}},
                    }},
                },
                "tasks": tasks,
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
