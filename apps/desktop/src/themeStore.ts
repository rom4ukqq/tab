export type ThemeInfo = { id: string; name: string; bytes: number };
type ThemeData = { columns: number; rows: number; frames: number; sourceFps: number; width: number; height: number; uncompressedBytes: number };
export type StoredTheme = ThemeInfo & { data: ThemeData; settings: Record<string, number | boolean | string> };
const LIMIT = 64 * 1024 * 1024;
const invalid = () => new Error('Invalid theme archive. Export it from TAB Theme Generator.');
const decoder = new TextDecoder('utf-8', { fatal: true });
export const validThemeId = (id: string | null): id is string => !!id && /^theme-[a-f0-9]{64}$/.test(id);

// The generator writes a stored ZIP container; frames inside it may be gzip-compressed.
// Read only exact, fixed filenames. No paths, scripts, ZIP64, encryption or ZIP inflation.
function readArchive(buffer: ArrayBuffer): Map<string, Uint8Array> {
  const bytes = new Uint8Array(buffer), view = new DataView(buffer), end = bytes.length - 22;
  const u16 = (n: number) => view.getUint16(n, true), u32 = (n: number) => view.getUint32(n, true);
  if (end < 0 || u32(end) !== 0x06054b50 || u16(end + 4) || u16(end + 6) || u16(end + 20)) throw invalid();
  const count = u16(end + 10), centralStart = u32(end + 16), centralSize = u32(end + 12);
  if (count < 2 || count > 5 || u16(end + 8) !== count || centralStart + centralSize !== end) throw invalid();
  const allowed = new Set(['theme.json', 'frames.bin.gz', 'frames.bin', 'preview.png', 'README.txt']);
  const entries = new Map<string, Uint8Array>(), ranges: [number, number][] = [];
  let at = centralStart;
  for (let i = 0; i < count; i++) {
    if (at + 46 > end || u32(at) !== 0x02014b50) throw invalid();
    const flags = u16(at + 8), method = u16(at + 10), packed = u32(at + 20), size = u32(at + 24);
    const nameLength = u16(at + 28), extra = u16(at + 30), comment = u16(at + 32), local = u32(at + 42);
    if (at + 46 + nameLength + extra + comment > end || method !== 0 || (flags & ~0x800) || packed !== size || size > LIMIT || u16(at + 34)) throw invalid();
    const name = decoder.decode(bytes.subarray(at + 46, at + 46 + nameLength));
    if (!allowed.has(name) || entries.has(name) || local + 30 > centralStart || u32(local) !== 0x04034b50) throw invalid();
    const localName = u16(local + 26), localExtra = u16(local + 28), start = local + 30 + localName + localExtra, stop = start + size;
    if (stop > centralStart || u16(local + 6) !== flags || u16(local + 8) !== method || u32(local + 18) !== size || u32(local + 22) !== size || u32(local + 14) !== u32(at + 16) || decoder.decode(bytes.subarray(local + 30, local + 30 + localName)) !== name) throw invalid();
    if (ranges.some(([a, b]) => local < b && stop > a)) throw invalid();
    ranges.push([local, stop]); entries.set(name, bytes.subarray(start, stop));
    at += 46 + nameLength + extra + comment;
  }
  if (at !== end) throw invalid();
  return entries;
}

function safeSettings(value: unknown): StoredTheme['settings'] {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw invalid();
  const source = value as Record<string, unknown>, result: StoredTheme['settings'] = {};
  const ranges: Record<string, [number, number]> = { speed: [0, 2], glyphSpeed: [0, 30], saturation: [0, 100], fontSize: [.5, 6], resolution: [20, 140], threshold: [0, 95], randomness: [0, 100] };
  for (const [key, [min, max]] of Object.entries(ranges)) {
    const v = source[key]; if (v === undefined) continue;
    if (typeof v !== 'number' || !Number.isFinite(v) || v < min || v > max) throw invalid(); result[key] = v;
  }
  for (const key of ['smooth', 'gradient', 'invert']) { const v = source[key]; if (v === undefined) continue; if (typeof v !== 'boolean') throw invalid(); result[key] = v; }
  for (const key of ['accent', 'secondary']) { const v = source[key]; if (v === undefined) continue; if (typeof v !== 'string' || !/^#[0-9a-f]{6}$/i.test(v)) throw invalid(); result[key] = v; }
  return result;
}

async function digest(bytes: ArrayBuffer): Promise<string> { return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)), b => b.toString(16).padStart(2, '0')).join(''); }
async function unpack(payload: Uint8Array, size: number, encoding: string): Promise<ArrayBuffer> {
  if (encoding === 'raw') { if (payload.length !== size) throw invalid(); return payload.slice().buffer; }
  const reader = new Blob([payload.slice().buffer]).stream().pipeThrough(new DecompressionStream('gzip')).getReader();
  const output = new Uint8Array(size); let offset = 0;
  try {
    while (true) { const { done, value } = await reader.read(); if (done) break; if (offset + value.length > size) throw invalid(); output.set(value, offset); offset += value.length; }
  } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
  if (offset !== size) throw invalid(); return output.buffer;
}

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open('tab-imported-themes', 1);
    request.onupgradeneeded = () => { request.result.createObjectStore('metadata', { keyPath: 'id' }); request.result.createObjectStore('frames'); };
    request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error('Theme storage is busy. Close other TAB windows.'));
  });
}
export async function listThemes(): Promise<ThemeInfo[]> {
  const db = await openDb();
  try { return await new Promise((resolve, reject) => { const tx = db.transaction('metadata'), request = tx.objectStore('metadata').getAll(); tx.oncomplete = () => resolve(request.result.map(({id, name, bytes}: ThemeInfo) => ({id, name, bytes}))); tx.onabort = () => reject(tx.error); }); }
  finally { db.close(); }
}
export async function loadTheme(id: string): Promise<{ manifest: StoredTheme; frames: ArrayBuffer }> {
  if (!validThemeId(id)) throw invalid();
  const db = await openDb();
  try {
    const stored = await new Promise<{ manifest: StoredTheme; blob: Blob }>((resolve, reject) => {
      const tx = db.transaction(['metadata', 'frames']), metadata = tx.objectStore('metadata').get(id), frames = tx.objectStore('frames').get(id);
      tx.oncomplete = () => metadata.result && frames.result ? resolve({ manifest: metadata.result, blob: frames.result }) : reject(new Error('Theme not found. Import it again.'));
      tx.onabort = () => reject(tx.error);
    });
    return { manifest: stored.manifest, frames: await stored.blob.arrayBuffer() };
  } finally { db.close(); }
}

export async function importTheme(file: File): Promise<ThemeInfo> {
  if (file.size > LIMIT) throw new Error('Theme archive exceeds 64 MiB.');
  const archive = await file.arrayBuffer(), entries = readArchive(archive), json = entries.get('theme.json');
  if (!json || json.length > 64 * 1024) throw invalid();
  const manifest = JSON.parse(decoder.decode(json)), d = manifest?.data;
  if (manifest.format !== 'agent-hub-ascii-theme' || manifest.version !== 1 || !d || d.layout !== 'frame-row-column-u8-luminance') throw invalid();
  if (typeof manifest.name !== 'string' || !manifest.name.trim() || manifest.name.length > 80) throw invalid();
  for (const key of ['columns', 'rows', 'frames', 'width', 'height', 'uncompressedBytes']) if (!Number.isSafeInteger(d[key]) || d[key] < 1) throw invalid();
  if (d.columns > 512 || d.rows > 512 || d.width > 16384 || d.height > 16384 || d.frames < 2 || d.frames > 50000 || typeof d.sourceFps !== 'number' || !Number.isFinite(d.sourceFps) || d.sourceFps <= 0 || d.sourceFps > 240 || d.uncompressedBytes > LIMIT || d.columns * d.rows * d.frames !== d.uncompressedBytes) throw invalid();
  if (!['gzip', 'raw'].includes(d.encoding) || d.path !== (d.encoding === 'gzip' ? 'frames.bin.gz' : 'frames.bin') || typeof d.sha256 !== 'string' || !/^[a-f0-9]{64}$/i.test(d.sha256)) throw invalid();
  const payload = entries.get(d.path); if (!payload) throw invalid();
  const frames = await unpack(payload, d.uncompressedBytes, d.encoding);
  if (await digest(frames) !== d.sha256.toLowerCase()) throw new Error('Theme checksum mismatch. Export the theme again.');
  const stored: StoredTheme = { id: `theme-${await digest(archive)}`, name: manifest.name.trim(), bytes: frames.byteLength, data: { columns: d.columns, rows: d.rows, frames: d.frames, width: d.width, height: d.height, sourceFps: d.sourceFps, uncompressedBytes: d.uncompressedBytes }, settings: safeSettings(manifest.settings) };
  const db = await openDb();
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(['metadata', 'frames'], 'readwrite'), metadata = tx.objectStore('metadata'), current = metadata.getAll();
      let limitError: Error | undefined;
      current.onsuccess = () => {
        const other = (current.result as ThemeInfo[]).filter(t => t.id !== stored.id);
        if (other.length >= 16 || other.reduce((sum, t) => sum + t.bytes, stored.bytes) > 256 * 1024 * 1024) { limitError = new Error('Theme storage limit reached (16 themes / 256 MiB).'); tx.abort(); return; }
        metadata.put(stored); tx.objectStore('frames').put(new Blob([frames]), stored.id);
      };
      tx.oncomplete = () => resolve(); tx.onabort = () => reject(limitError ?? tx.error ?? new Error('Could not save theme.'));
    });
  } finally { db.close(); }
  return { id: stored.id, name: stored.name, bytes: stored.bytes };
}
