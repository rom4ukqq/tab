import React, { useEffect, useRef, useState } from 'react';
import { MatrixBackdrop } from './CanvasMatrix';
import { loadTheme, validThemeId } from './themeStore';

export type BackgroundTheme = string;
export function validBackground(value: string | null): value is BackgroundTheme { return value === 'matrix' || validThemeId(value); }

export function ThemeBackdrop({ selected, animate, onError }: { selected: BackgroundTheme; animate: boolean; onError: (error: string) => void }) {
  const [shown, setShown] = useState<BackgroundTheme>('matrix');
  const [layers, setLayers] = useState<BackgroundTheme[]>([]);
  const frames = useRef(new Map<string, HTMLIFrameElement>());
  const pending = useRef(selected);
  pending.current = selected;
  useEffect(() => {
    if (selected === 'matrix') setShown('matrix');
    else setLayers(old => Array.from(new Set([...old.filter(id => id === shown), selected])));
  }, [selected]);
  useEffect(() => {
    const cleanup = window.setTimeout(() => setLayers(old => old.filter(id => id === shown || id === pending.current)), 800);
    return () => clearTimeout(cleanup);
  }, [shown]);
  useEffect(() => {
    const receive = (event: MessageEvent) => {
      const id = event.data?.theme;
      if (event.origin !== location.origin || event.source !== frames.current.get(id)?.contentWindow || id !== pending.current) return;
      if (event.data.type === 'themeReady') setShown(id);
      if (event.data.type === 'themeError') onError('Не удалось загрузить фон. Предыдущая тема сохранена.');
    };
    window.addEventListener('message', receive);
    return () => window.removeEventListener('message', receive);
  }, [onError]);
  useEffect(() => { frames.current.forEach(f => f.contentWindow?.postMessage({ type: 'motion', enabled: animate }, location.origin)); }, [animate, layers, shown]);
  const deliver = async (id: string) => {
    const frame = frames.current.get(id);
    try {
      const payload = await loadTheme(id);
      if (frames.current.get(id) !== frame || pending.current !== id) return;
      frame?.contentWindow?.postMessage({ type: 'themeData', theme: id, ...payload, enabled: animate }, location.origin, [payload.frames]);
    } catch { if (pending.current === id) onError('Не удалось загрузить фон. Предыдущая тема сохранена.'); }
  };
  return <div className="theme-background" aria-hidden="true">
    <div className="theme-layer" style={{ opacity: shown === 'matrix' ? 1 : 0 }}><MatrixBackdrop animate={animate && shown === 'matrix'}/></div>
    {layers.map(id => <iframe key={id} ref={element => { if (element) frames.current.set(id, element); else frames.current.delete(id); }} className="theme-layer" title="Imported ASCII background" tabIndex={-1} src={`/themes/player.html?theme=${encodeURIComponent(id)}`} style={{ opacity: shown === id ? 1 : 0 }} onLoad={() => void deliver(id)}/>) }
  </div>;
}
