"""Run private log capture without a Docker daemon; protect recovery scope."""
import os
import pathlib
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[2]
MARKER = "private-fixture-diagnostic-do-not-print"


class DeployRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.play = yaml.safe_load((ROOT / "ansible/deploy.yml").read_text())[0]
        roll = next(task for task in self.play["tasks"] if task.get("name") == "Roll the stack")
        self.rescue = roll["rescue"]

    def run_capture(self, broken_storage=False, hanging_logs=False):
        ansible = shutil.which("ansible-playbook")
        self.assertIsNotNone(ansible)
        with tempfile.TemporaryDirectory(prefix="mica-deploy-recovery-") as directory:
            path = pathlib.Path(directory)
            tools = path / "bin"
            tools.mkdir()
            docker = tools / "docker"
            tail = "sleep 60" if hanging_logs else "exit 1"
            docker.write_text(f"#!/bin/sh\nprintf '{MARKER}\\n' >&2\n{tail}\n")
            docker.chmod(0o700)
            if broken_storage:
                (path / "ops").write_text("storage unavailable")
            capture = next(task for task in self.rescue if "Preserve failed" in task["name"])
            # Resolve the real include while executing in an isolated fixture.
            capture["block"][0]["ansible.builtin.include_tasks"] = str(
                ROOT / "ansible/tasks/preserve-deploy-failure.yml"
            )
            fixture = [{
                "hosts": "localhost", "connection": "local", "gather_facts": False,
                "vars": {
                    "mica_dir": str(path), "version": "0.0.0",
                    "app_services": ["api", "web"],
                    "ansible_python_interpreter": sys.executable,
                },
                "environment": {"PATH": str(tools) + os.pathsep + os.environ["PATH"]},
                "tasks": [capture, {
                    "name": "Recovery can continue",
                    "ansible.builtin.debug": {"msg": "recovery-continues"},
                }],
            }]
            target = path / "check.yml"
            target.write_text(yaml.safe_dump(fixture, allow_unicode=True))
            result = subprocess.run(
                [ansible, "-i", "localhost,", str(target)],
                capture_output=True, text=True, timeout=60,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertNotIn(MARKER, result.stdout + result.stderr)
            self.assertIn("recovery-continues", result.stdout)
            if not broken_storage:
                evidence = list((path / "ops").glob("deploy-0.0.0-failed-*"))
                self.assertEqual(len(evidence), 1)
                self.assertEqual(stat.S_IMODE(evidence[0].stat().st_mode), 0o700)
                for service in ("api", "web"):
                    log = evidence[0] / (service + ".log")
                    self.assertIn(MARKER, log.read_text())
                    self.assertEqual(stat.S_IMODE(log.stat().st_mode), 0o600)

    def test_logs_retained_privately_before_recovery(self):
        self.run_capture()

    def test_storage_failure_does_not_block_recovery(self):
        self.run_capture(broken_storage=True)

    def test_log_reader_timeout_does_not_block_recovery(self):
        self.run_capture(hanging_logs=True)

    def test_rollback_targets_only_application_services(self):
        rollback = next(task for task in self.rescue if task.get("name") == "Best-effort restart on the previous version")
        args = rollback["community.docker.docker_compose_v2"]
        self.assertEqual(args["services"], "{{ app_services + (sidecars_present | default([])) }}")
        self.assertIs(args["dependencies"], False)
        render = next(task for task in self.play["tasks"] if task.get("name") == "Render .env")
        self.assertIs(render["diff"], False)


if __name__ == "__main__":
    unittest.main()
