"""Import a reviewed theme archive; only fixed data files are copied, never scripts."""
from pathlib import Path
import gzip
import hashlib
import json
import sys
import zipfile

archive, theme_id = sys.argv[1:]
if theme_id not in ('saveli', 'pr0n'):
    raise ValueError('Unknown bundled theme')
with zipfile.ZipFile(archive) as z:
    if len(z.namelist()) != len(set(z.namelist())):
        raise ValueError('Duplicate ZIP entries')
    for info in z.infolist():
        if info.file_size > 64 * 1024 * 1024:
            raise ValueError('Entry too large')
    manifest = json.loads(z.read('theme.json'))
    assert manifest['format'] == 'agent-hub-ascii-theme' and manifest['version'] == 1
    data = manifest['data']
    assert data['encoding'] == 'gzip' and data['path'] == 'frames.bin.gz'
    assert data['uncompressedBytes'] <= 64 * 1024 * 1024
    payload = z.read('frames.bin.gz')
    import io
    with gzip.GzipFile(fileobj=io.BytesIO(payload)) as stream:
        raw = stream.read(data['uncompressedBytes'] + 1)
    assert len(raw) == data['uncompressedBytes'] == data['columns'] * data['rows'] * data['frames']
    assert hashlib.sha256(raw).hexdigest() == data['sha256']
    destination = Path(__file__).resolve().parents[2] / 'assets/themes' / theme_id
    destination.mkdir(parents=True, exist_ok=True)
    (destination / 'theme.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding='utf-8')
    (destination / 'frames.bin.gz').write_bytes(payload)
    (destination / 'preview.png').write_bytes(z.read('preview.png'))
    print(f'{theme_id}: checksum and dimensions verified, {len(raw)} decoded bytes')
