#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["pyte==0.8.2", "pillow>=11,<13"]
# ///
"""Record a real, read-only TUI session on generated files (POSIX + ffmpeg).

Run with uv run scripts/record-demo.py --binary /path/to/cleanr --output docs/static.
Requires a monospaced font; pass --font on non-macOS hosts. No cleanup keys are sent.
The .cast retains the actual ANSI output; PNG/MP4 render it with pyte and Pillow.
"""
import argparse
import codecs
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import tempfile
import termios
import time

from PIL import Image, ImageDraw, ImageFont
import pyte

COLS, ROWS, WIDTH, HEIGHT, FPS = 120, 30, 1200, 720, 5
BACKGROUND, FOREGROUND = '#101820', '#e6edf3'
PALETTE = dict(black='#101820', red='#ef807d', green='#9ee3b4', brown='#eccf94',
               blue='#8ac3f2', magenta='#d6abef', cyan='#89d9d7', white='#e6edf3',
               brightblack='#8493a5', brightred='#ffada6', brightgreen='#b4edc5',
               brightbrown='#fce4af', brightblue='#b0d9fc', brightmagenta='#e6c9f7',
               brightcyan='#b1e7e5', brightwhite='#ffffff')


def color(value, default):
    if value == 'default':
        return default
    return PALETTE.get(value, '#' + value if len(value) == 6 else default)


def render(screen, font, label, elapsed):
    frame = Image.new('RGB', (WIDTH, HEIGHT), BACKGROUND)
    draw = ImageDraw.Draw(frame)
    draw.text((24, 16), label, font=font, fill='#9ee3b4')
    draw.line((24, 47, WIDTH - 24, 47), fill='#31404d')
    for row in range(ROWS):
        for col in range(COLS):
            char = screen.buffer[row][col]
            fg, bg = color(char.fg, FOREGROUND), color(char.bg, BACKGROUND)
            if char.reverse:
                fg, bg = bg, fg
            x, y = 24 + col * 9.6, 62 + row * 20
            if bg != BACKGROUND:
                draw.rectangle((x, y, x + 10, y + 20), fill=bg)
            if char.data.strip():
                draw.text((x, y), char.data, font=font, fill=fg)
    draw.text((24, 687), f'Generated samples | Read-only scan and review | {elapsed:02.0f}s', font=font, fill='#a8b7c6')
    return frame


def snapshot(root):
    return {str(p.relative_to(root)): (hashlib.sha256(p.read_bytes()).hexdigest(), p.stat().st_mtime_ns)
            for p in root.rglob('*') if p.is_file()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--font', default='/System/Library/Fonts/Menlo.ttc')
    args = parser.parse_args()
    binary, output = args.binary.resolve(strict=True), args.output.resolve()
    font = ImageFont.truetype(args.font, 16)
    version = subprocess.check_output([str(binary), '--version'], text=True).strip()
    for name in ['img', 'media']:
        (output / name).mkdir(parents=True, exist_ok=True)
    events, preview_saved = [], False
    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.Stream(screen)
    decoder = codecs.getincrementaldecoder('utf-8')('replace')
    with tempfile.TemporaryDirectory(prefix='cleanr-demo-') as tmp:
        root = Path(tmp)
        projects = root / 'projects'
        fixtures = {'web-app/node_modules/example/cache.bin': 16, 'rust-tool/target/debug/cache.bin': 8,
                    'site/dist/bundle.js': 4}
        stamp = time.time() - 120 * 86400
        for name, size in fixtures.items():
            file = projects / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(b'0' * (size * 1024 * 1024))
        for name, contents in [('web-app/package.json', '{"name":"example-web","private":true}'),
                               ('rust-tool/Cargo.toml', '[package]\nname="example-cli"\nversion="0.1.0"\n'),
                               ('site/package.json', '{"name":"example-site","private":true}')]:
            (projects / name).write_text(contents)
        for file in sorted(projects.rglob('*'), reverse=True):
            os.utime(file, (stamp, stamp))
        before = snapshot(projects)
        config = root / 'demo.toml'
        config.write_text('[ui]\ntheme="dark"\n[i18n]\nlocale="en-US"\ndirs=[]\n[plugins]\ndirs=[]\n')
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', ROWS, COLS, 0, 0))
        child_env = {**os.environ, 'TERM': 'xterm-256color', 'COLORTERM': 'truecolor', 'CLEANR_NO_UPDATE_CHECK': 'true'}
        child = subprocess.Popen([str(binary), '--no-update-check', '--config', str(config), 'projects'],
                                 cwd=root, env=child_env, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
        os.close(slave)
        encoder = subprocess.Popen(['ffmpeg', '-y', '-loglevel', 'error', '-f', 'rawvideo', '-pix_fmt', 'rgb24',
                                    '-s', f'{WIDTH}x{HEIGHT}', '-r', str(FPS), '-i', '-', '-an', '-c:v', 'libx264',
                                    '-preset', 'fast', '-crf', '22', '-pix_fmt', 'yuv420p', '-movflags', '+faststart',
                                    str(output / 'media/cleanr-first-scan.mp4')], stdin=subprocess.PIPE)
        # These inputs only scan, navigate, display help, and quit. Never add clean/restore input.
        inputs = iter([(3, 's'), (12, 'r'), (18, 'j'), (22, 'k'), (26, '?'), (30, '\x1b'), (34, 'q')])
        next_input = next(inputs, None)
        start, next_frame = time.monotonic(), 0.0
        try:
            while True:
                elapsed = time.monotonic() - start
                if elapsed > 36:
                    break
                if next_input and elapsed >= next_input[0]:
                    os.write(master, next_input[1].encode())
                    events.append([round(elapsed, 4), 'i', next_input[1]])
                    next_input = next(inputs, None)
                if select.select([master], [], [], 0.02)[0]:
                    try:
                        data = os.read(master, 65536)
                    except OSError:
                        break
                    if not data:
                        break
                    text = decoder.decode(data)
                    events.append([round(elapsed, 4), 'o', text])
                    stream.feed(text)
                    if '\x1b[6n' in text:
                        os.write(master, b'\x1b[1;1R')
                    if '\x1b[c' in text:
                        os.write(master, b'\x1b[?1;2c')
                if elapsed >= next_frame:
                    frame = render(screen, font, f'{version} | Terminal recording', elapsed)
                    encoder.stdin.write(frame.tobytes())
                    if elapsed >= 9 and not preview_saved:
                        frame.save(output / 'img/cleanr-scan.png')
                        (output / 'media/cleanr-scan.txt').write_text('\n'.join(screen.display) + '\n')
                        preview_saved = True
                    next_frame += 1 / FPS
        finally:
            if child.poll() is None:
                child.terminate()
            child.wait(timeout=5)
            os.close(master)
            encoder.stdin.close()
            if encoder.wait(timeout=30) != 0:
                raise RuntimeError('ffmpeg encoding failed')
        if not preview_saved:
            raise RuntimeError('TUI ended before a scan frame was captured')
        if snapshot(projects) != before:
            raise RuntimeError('Read-only demonstration changed its sample files')
        # Reports must prove a completed scan, not just a successful recorder process.
        visible = (output / 'media/cleanr-scan.txt').read_text()
        if 'Scan Tree' not in visible or '24.00 MiB' not in visible:
            raise RuntimeError('Expected scan result is missing; inspect the terminal transcript')
    header = {'version': 2, 'width': COLS, 'height': ROWS, 'title': f'{version}: first read-only scan',
              'env': {'TERM': 'xterm-256color'}}
    (output / 'media/cleanr-first-scan.cast').write_text('\n'.join(json.dumps(row) for row in [header, *events]) + '\n')
    metadata = {'version': version, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                'platform': os.uname().sysname + ' ' + os.uname().machine, 'duration_seconds': events[-1][0],
                'sample_files_unchanged': True, 'operations': ['scan', 'review', 'navigate', 'help', 'quit'],
                'terminal': {'columns': COLS, 'rows': ROWS}, 'renderer': 'pyte + Pillow; original ANSI in .cast'}
    (output / 'media/cleanr-demo.json').write_text(json.dumps(metadata, indent=2) + '\n')
    print(json.dumps(metadata))


if __name__ == '__main__':
    main()
