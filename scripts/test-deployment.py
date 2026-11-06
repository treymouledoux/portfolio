"""Exercise activation/rollback in temporary directories, never real services."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile

SCRIPT = Path(__file__).resolve().parents[1] / 'deploy/activate-release.sh'
SHIM = r'''#!/usr/bin/env python3
import fcntl, hashlib, os, pathlib, subprocess, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
if name == 'sudo':
    sys.exit(subprocess.call(args))
elif name == 'systemctl':
    with open(os.environ['MOCK_CALLS'], 'a') as f: f.write(' '.join(args) + '\n')
elif name == 'curl':
    if os.environ.get('FAIL_HEALTH') == '1': sys.exit(22)
    print('ok')
elif name == 'flock':
    fcntl.flock(int(args[-1]), fcntl.LOCK_EX | fcntl.LOCK_NB)
elif name == 'mv':
    paths = [a for a in args if not a.startswith('-')]
    os.replace(*paths)
elif name == 'sha256sum':
    for line in pathlib.Path(args[1]).read_text().splitlines():
        digest, path = line.split('  ', 1)
        if hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest() != digest: sys.exit(1)
elif name not in ('sleep','sync'):
    raise RuntimeError(name)
'''

for scenario in ('success', 'rollback', 'legacy-rollback', 'bad-upload'):
    with tempfile.TemporaryDirectory(prefix='portfolio-deploy-test-') as directory:
        base = Path(directory)
        commands = base / 'commands'
        commands.mkdir()
        for name in ('sudo','systemctl','curl','flock','mv','sha256sum','sleep','sync'):
            p = commands / name
            p.write_text(SHIM)
            p.chmod(0o755)
        unit_dir = base / 'units'
        unit_dir.mkdir()
        (unit_dir / 'portfolio.service').write_text('previous service unit\n')
        (unit_dir / 'cloudflared.service').write_text('existing tunnel unit\n')
        old = base / 'releases/old'
        old.mkdir(parents=True)
        if scenario != 'legacy-rollback': (base / 'current').symlink_to(old)
        legacy = '{"message":"preserve me"}\n'
        (base / 'submissions.json').write_text(legacy)
        release = base / 'releases/new'
        (release / 'assets').mkdir(parents=True)
        for name, content in {'portfolio-serve': 'binary fixture', 'portfolio.service': 'new unit', 'cloudflared.service': 'tunnel unit', 'assets/index.html': '<html>new release</html>', 'assets/resume.pdf': '%PDF-fixture'}.items():
            (release / name).write_text(content)
        (release / 'SHA256SUMS').write_text(''.join(f'{hashlib.sha256(p.read_bytes()).hexdigest()}  ./{p.relative_to(release)}\n' for p in sorted(release.rglob('*')) if p.is_file()))
        if scenario == 'bad-upload': (release / 'portfolio-serve').write_text('damaged upload')
        env = dict(os.environ, PATH=str(commands)+os.pathsep+os.environ['PATH'], PORTFOLIO_UNIT_DIR=str(unit_dir), MOCK_CALLS=str(base/'calls'), FAIL_HEALTH='1' if 'rollback' in scenario else '0')
        result = subprocess.run(['bash',str(SCRIPT),str(base),'new'],env=env,capture_output=True,text=True,timeout=20)
        calls = (base/'calls').read_text() if (base/'calls').exists() else ''
        assert (base/'submissions.json').read_text() == legacy
        assert (unit_dir/'cloudflared.service').read_text() == 'existing tunnel unit\n'
        if scenario == 'success':
            assert result.returncode == 0, result.stderr
            assert (base/'current').resolve() == release.resolve()
            assert (unit_dir/'portfolio.service').read_text() == 'new unit'
            assert (base/'data/submissions.jsonl').read_text() == legacy
        else:
            assert result.returncode != 0
            assert (unit_dir/'portfolio.service').read_text() == 'previous service unit\n'
            if scenario == 'legacy-rollback': assert not (base/'current').exists()
            else: assert (base/'current').resolve() == old.resolve()
            if scenario == 'bad-upload': assert 'stop portfolio.service' not in calls
            else:
                assert 'start portfolio.service' in calls
                assert (base/'data/submissions.jsonl').read_text() == legacy
        assert 'stop cloudflared.service' not in calls
        print('PASS', scenario)
