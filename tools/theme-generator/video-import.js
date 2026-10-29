/* Sequential local video import. Mediabunny is vendored; no CDN or uploads. */
window.decodeAsciiVideo = async function decodeAsciiVideo(file, { signal, onProgress = () => {} } = {}) {
  if (!window.VideoDecoder || !window.Mediabunny) {
    throw Error('Последовательный декодер недоступен. Открой генератор в актуальном Chrome или Edge.');
  }
  const { Input, BlobSource, ALL_FORMATS, VideoSampleSink } = window.Mediabunny;
  const input = new Input({ source: new BlobSource(file), formats: ALL_FORMATS });
  const checkAbort = () => { if (signal?.aborted) throw new DOMException('Import cancelled', 'AbortError'); };
  const abort = () => input.dispose();
  signal?.addEventListener('abort', abort, { once: true });
  try {
    checkAbort();
    const track = await input.getPrimaryVideoTrack();
    if (!track || !await track.canDecode()) throw Error('Браузер не поддерживает декодирование этого видео. Попробуй MP4 H.264 или WebM.');
    const [first, end, width, height] = await Promise.all([
      track.getFirstTimestamp(), track.computeDuration(), track.getDisplayWidth(), track.getDisplayHeight(),
    ]);
    checkAbort();
    const start = Math.max(0, first), duration = end - start;
    if (!Number.isFinite(duration) || duration < .25 || duration > 180) throw Error('Выбери ролик длительностью от 0,25 секунды до 3 минут.');
    if (!(width > 0 && height > 0)) throw Error('Некорректный размер видео.');
    // Keep the previous import's exact 24 Hz grid and luminance resolution.
    const fps = 24, count = Math.max(2, Math.floor(duration * fps));
    const scale = Math.min(90 / width, 68 / height);
    const columns = Math.max(2, Math.round(width * scale)), rows = Math.max(2, Math.round(height * scale));
    const size = columns * rows, data = new Uint8Array(count * size);
    const canvas = document.createElement('canvas'); canvas.width = columns; canvas.height = rows;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    const sink = new VideoSampleSink(track);
    function* timestamps() { for (let i = 0; i < count; i++) yield start + i / fps; }
    let index = 0, lastYield = performance.now();
    // Sorted timestamps use one forward decoding pipeline: each packet is decoded at most once.
    // Unlike real-time playback, this does not skip frames when rendering cannot keep up.
    for await (const sample of sink.samplesAtTimestamps(timestamps())) {
      try {
        checkAbort();
        if (!sample) throw Error('В ролике отсутствует запрошенный кадр.');
        context.clearRect(0, 0, columns, rows);
        sample.draw(context, 0, 0, columns, rows);
        const pixels = context.getImageData(0, 0, columns, rows).data;
        for (let cell = 0; cell < size; cell++) data[index * size + cell] = Math.round(pixels[cell * 4] * .299 + pixels[cell * 4 + 1] * .587 + pixels[cell * 4 + 2] * .114);
        index++;
      } finally { sample?.close(); }
      if (performance.now() - lastYield > 50 || index === count) {
        onProgress(index / count);
        await new Promise(resolve => setTimeout(resolve, 0));
        checkAbort(); lastYield = performance.now();
      }
    }
    if (index !== count) throw Error('Декодирование завершилось до конца ролика.');
    return { data, columns, rows, count, fps, width, height };
  } finally {
    signal?.removeEventListener('abort', abort);
    input.dispose();
  }
};
