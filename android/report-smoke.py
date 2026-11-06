"""Expose concise runtime failures through GitHub annotations, not just artifacts."""
import pathlib
import re
import xml.etree.ElementTree as ET


def emit(message):
    # Keep below GitHub's per-annotation truncation, preserving the useful start.
    message = message[:3000].replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
    print(f'::error title=Android runtime smoke::{message}', flush=True)


for report in pathlib.Path('android/app/build/outputs/androidTest-results').rglob('*.xml'):
    try:
        root = ET.parse(report).getroot()
    except ET.ParseError:
        continue
    for failure in root.iter('failure'):
        emit(f'{report.name}\n{failure.get("message", "")}\n{failure.text or ""}')

logcat = pathlib.Path('emulator-logcat.txt')
if logcat.exists():
    lines = [line for line in logcat.read_text(errors='replace').splitlines()
             if re.search(r'AndroidRuntime|ExoPlayerImplInternal|PlaybackException|panicked|System\.err', line)]
    # Several short messages retain relevant tail lines without annotation truncation.
    for offset in range(max(0, len(lines) - 40), len(lines), 8):
        emit('\n'.join(lines[offset:offset + 8]))
