"""Reproducible platform packages; never copy local logs or credentials."""
import argparse
import hashlib
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / 'releases'
PYTHON_VERSION = '3.14.8'
WINDOWS_SHA256 = 'a93abe456ab01bd96d7a085b3cdb6566b3063f4241360d114142fbdb07f0a310'


def copy_core(destination):
    shutil.copytree(ROOT / 'BUAASrunLogin', destination / 'BUAASrunLogin',
                    ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
    shutil.copy2(ROOT / 'README.md', destination / 'README.md')


def build_macos():
    if sys.platform != 'darwin' or platform.machine() != 'arm64':
        raise SystemExit('Mac ARM 版须在 Apple Silicon Mac 上构建')
    subprocess.run([sys.executable, '-m', 'PyInstaller', '--noconfirm', '--clean',
                    '--windowed', '--target-arch', 'arm64', '--name', 'BUAALogin',
                    '--osx-bundle-identifier', 'cn.buaa.autologin', 'desktop.py'],
                   cwd=ROOT, check=True)
    app = ROOT / 'dist' / 'BUAALogin.app'
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True)
    archive = OUTPUT / 'BUAALogin-Mac-arm64.zip'
    if archive.exists():
        archive.unlink()
    subprocess.run(['ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', str(app), str(archive)], check=True)
    return archive


def build_windows(runtime=None):
    stage = ROOT / 'build' / 'windows' / 'BUAALogin'
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    copy_core(stage)
    shutil.copy2(ROOT / 'desktop_worker.py', stage)
    for file in (ROOT / 'platform' / 'windows').iterdir():
        shutil.copy2(file, stage / file.name)
    if runtime:
        runtime = Path(runtime)
    else:
        runtime = ROOT / 'build' / f'python-{PYTHON_VERSION}-embed-amd64.zip'
        urllib.request.urlretrieve(f'https://www.python.org/ftp/python/{PYTHON_VERSION}/python-{PYTHON_VERSION}-embed-amd64.zip', runtime)
    if hashlib.sha256(runtime.read_bytes()).hexdigest() != WINDOWS_SHA256:
        raise SystemExit('Windows Python 运行时校验失败')
    with zipfile.ZipFile(runtime) as archive:
        archive.testzip()
        archive.extractall(stage / 'runtime')
    # The embedded runtime uses an isolated path file; explicitly include the app.
    (stage / 'runtime' / 'python314._pth').write_text('python314.zip\n.\n..\n', encoding='utf-8')
    output = OUTPUT / 'BUAALogin-Windows-x64.zip'
    with zipfile.ZipFile(output, 'w', zipfile.ZIP_DEFLATED) as archive:
        for file in stage.rglob('*'):
            if file.is_file():
                archive.write(file, file.relative_to(stage.parent))
    return output


def build_docker():
    stage = ROOT / 'build' / 'docker' / 'BUAALogin-Docker'
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    copy_core(stage)
    for name in ('always_online.py', 'docker_entry.py', 'Dockerfile', '.dockerignore'):
        shutil.copy2(ROOT / name, stage / name)
    (stage / 'run.sh').write_text('#!/bin/sh\nset -eu\n: "${USERNAME:?请设置 USERNAME}"\n: "${PASSWORD:?请设置 PASSWORD}"\nexport USERNAME PASSWORD\ncd "$(dirname "$0")"\ndocker build -t buaalogin:local .\nexec docker run --rm --network host -e USERNAME -e PASSWORD buaalogin:local "$@"\n')
    (stage / 'run.sh').chmod(0o755)
    output = OUTPUT / 'BUAALogin-Docker.tar.gz'
    with tarfile.open(output, 'w:gz') as archive:
        archive.add(stage, arcname=stage.name)
    return output


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('platform', choices=['macos', 'windows', 'docker'])
    parser.add_argument('--windows-runtime', help='已下载的官方 Python 嵌入式运行时 ZIP')
    args = parser.parse_args()
    OUTPUT.mkdir(exist_ok=True)
    artifact = build_windows(args.windows_runtime) if args.platform == 'windows' else (
        build_macos() if args.platform == 'macos' else build_docker())
    print(artifact)


if __name__ == '__main__':
    main()
