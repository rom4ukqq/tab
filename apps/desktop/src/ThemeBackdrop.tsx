import React, { useEffect, useRef, useState } from 'react';
import { MatrixBackdrop } from './WindowChrome';

export const backgroundThemes = [{ id: 'matrix', name: 'Матрица' }, { id: 'saveli', name: 'Saveli' }, { id: 'pr0n', name: 'pr0n' }] as const;
export type BackgroundTheme = typeof backgroundThemes[number]['id'];
export function validBackground(value: string | null): value is BackgroundTheme { return backgroundThemes.some(t => t.id === value); }

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
  useEffect(() => {
    let drawing = false;
    const send = (type: string, event?: PointerEvent) => frames.current.get(shown)?.contentWindow?.postMessage({ type, x: event ? event.clientX / innerWidth : 0, y: event ? event.clientY / innerHeight : 0 }, location.origin);
    const down = (e: PointerEvent) => {
      if (e.button !== 0 || (e.target as HTMLElement).closest('button,input,select,textarea,a,label,.card,aside')) return;
      drawing = true; send('brush', e);
    };
    const move = (e: PointerEvent) => { if (drawing && (e.buttons & 1)) send('brush', e); else if (drawing) up(); };
    const up = () => { drawing = false; send('brushEnd'); };
    window.addEventListener('pointerdown', down); window.addEventListener('pointermove', move); window.addEventListener('pointerup', up); window.addEventListener('blur', up);
    return () => { window.removeEventListener('pointerdown', down); window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); window.removeEventListener('blur', up); };
  }, [shown]);
  return <div className="theme-background" aria-hidden="true">
    <div className="theme-layer" style={{ opacity: shown === 'matrix' ? 1 : 0 }}><MatrixBackdrop animate={animate && shown === 'matrix'}/></div>
    {layers.map(id => <iframe key={id} ref={element => { if (element) frames.current.set(id, element); else frames.current.delete(id); }} className="theme-layer" title={`${id} ASCII background`} tabIndex={-1} src={`/themes/player.html?theme=${id}`} style={{ opacity: shown === id ? 1 : 0 }} onLoad={() => frames.current.get(id)?.contentWindow?.postMessage({ type: 'motion', enabled: animate }, location.origin)}/>) }
  </div>;
}
