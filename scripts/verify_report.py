"""Run the report's checks and preserve their actual output, without changing team data."""
import json
import pathlib
import platform
import subprocess

root = pathlib.Path(__file__).resolve().parents[1]
out = root / 'docs/evidence/2026-09-05'
out.mkdir(parents=True, exist_ok=True)
commands = [
    ['rustc', '--version'], ['cargo', '--version'],
    ['cargo', 'fmt', '--check'],
    ['cargo', 'clippy', '--offline', '--all-targets', '--', '-D', 'warnings'],
    ['cargo', 'test', '--offline'], ['cargo', 'build', '--release', '--offline'],
]
records = []
for command in commands:
    r = subprocess.run(command, cwd=root, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    records.append({'command': ' '.join(command), 'exit_code': r.returncode, 'output': r.stdout})
    if r.returncode:
        raise SystemExit(r.stdout)
(out / 'checks.json').write_text(json.dumps({'platform': platform.platform(), 'checks': records}, ensure_ascii=False, indent=2))
(out / 'checks.txt').write_text('\n\n'.join('$ ' + r['command'] + '\n' + r['output'] + f'Exit code: {r["exit_code"]}' for r in records))
print('All checks passed; logs saved in', out)
