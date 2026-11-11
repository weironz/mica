#!/usr/bin/env python3
"""Resolve release CLI runners before scheduling any optional macOS runner."""
import json
import os
import sys

raw = os.environ.get('BUILD_MACOS_CLI', '').strip().lower()
if raw not in ('', 'false', 'true'):
    sys.exit('BUILD_MACOS_CLI must be true or false')
entries = [
    {'os': 'windows-latest', 'bin': 'mica-cli.exe', 'suffix': 'windows-x64.exe', 'osname': 'windows'},
    # Static musl is the existing portability contract, not the runner's glibc.
    {'os': 'ubuntu-latest', 'bin': 'mica-cli', 'suffix': 'linux-x64',
     'target': 'x86_64-unknown-linux-musl', 'osname': 'linux'},
]
if raw == 'true':
    entries.append({'os': 'macos-latest', 'bin': 'mica-cli', 'suffix': 'macos-arm64', 'osname': 'macos'})
print('matrix=' + json.dumps({'include': entries}, separators=(',', ':')))
